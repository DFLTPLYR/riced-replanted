use std::{
    collections::{HashMap, HashSet},
    env,
    io::{self, BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

pub mod protocol;

pub use protocol::{MethodSpec, ToHost, ToPlugin, VERSION};

pub type Handler = Box<dyn Fn(&Value) -> Result<Value, String> + Send + Sync>;

pub struct Plugin {
    name: String,
    state: Vec<String>,
    methods: Vec<(String, MethodSpec, Handler)>,
}

impl Plugin {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            state: Vec::new(),
            methods: Vec::new(),
        }
    }

    pub fn state<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.state = keys.into_iter().map(Into::into).collect();
        self
    }

    pub fn method<F>(mut self, name: &str, readonly: bool, handler: F) -> Self
    where
        F: Fn(&Value) -> Result<Value, String> + Send + Sync + 'static,
    {
        self.methods
            .push((name.to_owned(), MethodSpec { readonly }, Box::new(handler)));
        self
    }

    /// Connects to the socket in `RICED_SOCKET`, registers, and runs `run` on this thread.
    ///
    /// The process exits when the host disconnects, so a plugin never outlives riced.
    pub fn serve<F>(self, run: F) -> io::Result<()>
    where
        F: FnOnce(Context),
    {
        let socket = env::var_os("RICED_SOCKET")
            .map(PathBuf::from)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "RICED_SOCKET is not set; start the extension from riced or set it",
                )
            })?;
        serve_at(self, &socket, run)
    }
}

/// Context handed to a running extension.
pub struct Context {
    tx: mpsc::Sender<Vec<u8>>,
    requested: Arc<Mutex<HashSet<String>>>,
}

impl Context {
    /// Fields whose store table has been read by Lua, in sorted order.
    pub fn requested(&self) -> Vec<String> {
        let mut fields = self
            .requested
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        fields.sort();
        fields
    }

    /// Whether Lua has read this store field since the extension started.
    pub fn wants(&self, field: &str) -> bool {
        self.requested.lock().unwrap().contains(field)
    }

    /// Replaces the watched snapshot. Lua reads it as `riced.plugins(name).store`.
    pub fn publish(&self, store: Value) {
        self.send(ToHost::State { store });
    }

    /// Pushes an event into `app:update` as `{ type = "plugin", plugin = ..., event = ... }`.
    pub fn event(&self, event: Value) {
        self.send(ToHost::Event { event });
    }

    fn send(&self, message: ToHost) {
        match encode(&message) {
            Ok(line) => {
                let _ = self.tx.send(line);
            }
            Err(error) => eprintln!("riced extension: {error}"),
        }
    }
}

pub fn serve_at<F>(plugin: Plugin, socket: &Path, run: F) -> io::Result<()>
where
    F: FnOnce(Context),
{
    if plugin.name.is_empty()
        || plugin.name.len() > 32
        || !plugin
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "plugin name must match [a-z0-9_-]{1,32}",
        ));
    }

    let mut writer = connect(socket)?;
    let reader = writer.try_clone()?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::Builder::new()
        .name("riced-extension-writer".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            for line in rx {
                if writer.write_all(&line).is_err() {
                    break;
                }
            }
        })?;

    let Plugin {
        name,
        state,
        methods: definitions,
    } = plugin;
    let mut methods = HashMap::new();
    let mut specs = HashMap::new();
    for (method, spec, handler) in definitions {
        methods.insert(method.clone(), handler);
        specs.insert(method, spec);
    }
    let register = ToHost::Register {
        v: VERSION,
        name,
        state,
        methods: specs,
        pid: Some(std::process::id()),
    };
    tx.send(encode(&register)?)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "riced closed the connection"))?;

    let replies = tx.clone();
    let requested = Arc::new(Mutex::new(HashSet::new()));
    let reader_requested = Arc::clone(&requested);
    thread::Builder::new()
        .name("riced-extension-reader".into())
        .stack_size(256 * 1024)
        .spawn(move || read_calls(BufReader::new(reader), methods, replies, reader_requested))?;

    run(Context { tx, requested });
    Ok(())
}

fn connect(path: &Path) -> io::Result<UnixStream> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(stream),
            Err(error) if Instant::now() < deadline => {
                if error.kind() == io::ErrorKind::PermissionDenied {
                    return Err(error);
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error),
        }
    }
}

fn read_calls(
    mut reader: BufReader<UnixStream>,
    methods: HashMap<String, Handler>,
    tx: mpsc::Sender<Vec<u8>>,
    requested: Arc<Mutex<HashSet<String>>>,
) {
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) if line.len() > protocol::MAX_LINE => {
                eprintln!("riced extension: message over {} bytes", protocol::MAX_LINE);
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
        let message = match serde_json::from_slice::<ToPlugin>(&line) {
            Ok(message) => message,
            Err(error) => {
                eprintln!("riced extension: {error}");
                continue;
            }
        };
        match message {
            ToPlugin::Call { id, method, args } => {
                let result = match methods.get(&method) {
                    Some(handler) => handler(&Value::Array(args)),
                    None => Err(format!("unknown method: {method}")),
                };
                let reply = match result {
                    Ok(result) => ToHost::Result {
                        id,
                        ok: true,
                        result,
                        error: None,
                    },
                    Err(error) => ToHost::Result {
                        id,
                        ok: false,
                        result: Value::Null,
                        error: Some(error),
                    },
                };
                match encode(&reply) {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(error) => eprintln!("riced extension: {error}"),
                }
            }
            ToPlugin::Subscribe { fields } => {
                requested.lock().unwrap().extend(fields);
            }
            ToPlugin::Error { error } => {
                eprintln!("riced extension: {error}");
                std::process::exit(1);
            }
        }
    }
    // riced went away, or the host rejected the registration.
    std::process::exit(0);
}

fn encode(message: &ToHost) -> io::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if line.len() + 1 > protocol::MAX_LINE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message exceeds the 256 KiB line limit",
        ));
    }
    line.push(b'\n');
    Ok(line)
}

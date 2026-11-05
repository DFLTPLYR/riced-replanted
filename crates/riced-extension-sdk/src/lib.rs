use std::{
    collections::{HashMap, HashSet},
    env,
    io::{self, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

pub mod logging;
pub mod protocol;

pub use log;
pub use protocol::{MethodSpec, ToHost, ToPlugin, VERSION};

pub type Handler = Box<dyn Fn(&Value) -> Result<Value, String> + Send + Sync>;

/// Keeps complete array entries within a JSON byte budget, including escaping.
/// Oversized entries are omitted; identifiers and action keys are never rewritten.
pub fn bounded_array(items: impl IntoIterator<Item = Value>, budget: usize) -> Vec<Value> {
    let mut remaining = budget.saturating_sub(2);
    let mut omitted = false;
    let items = items
        .into_iter()
        .filter(|item| {
            let bytes = item.to_string().len() + 1;
            if bytes > remaining {
                omitted = true;
                false
            } else {
                remaining -= bytes;
                true
            }
        })
        .collect();
    if omitted {
        log::warn!(target: "extension.snapshot", "snapshot array exceeded its byte budget; entries omitted");
    }
    items
}

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
            Err(error) => log::error!(target: "extension.encode", "{error}"),
        }
    }
}

pub fn serve_at<F>(plugin: Plugin, socket: &Path, run: F) -> io::Result<()>
where
    F: FnOnce(Context),
{
    let _ = logging::init();
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
        match protocol::read_line(&mut reader, &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                log::error!(target: "extension.protocol", "{error}");
                break;
            }
        }
        let message = match serde_json::from_slice::<ToPlugin>(&line) {
            Ok(message) => message,
            Err(error) => {
                log::warn!(target: "extension.decode", "{error}");
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
                    Err(error) => log::error!(target: "extension.encode", "{error}"),
                }
            }
            ToPlugin::Subscribe { fields } => {
                requested.lock().unwrap().extend(fields);
            }
            ToPlugin::Error { error } => {
                log::error!(target: "extension.rejected", "{error}");
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
        if let ToHost::Result { id, .. } = message {
            return encode(&ToHost::Result {
                id: *id,
                ok: false,
                result: Value::Null,
                error: Some("reply exceeds the 256 KiB line limit".into()),
            });
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message exceeds the 256 KiB line limit",
        ));
    }
    line.push(b'\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn oversized_replies_return_a_bounded_error_with_the_original_id() {
        for oversized in [
            ToHost::Result {
                id: 42,
                ok: true,
                result: Value::String("x".repeat(protocol::MAX_LINE)),
                error: None,
            },
            ToHost::Result {
                id: 42,
                ok: false,
                result: Value::Null,
                error: Some("x".repeat(protocol::MAX_LINE)),
            },
        ] {
            let bytes = encode(&oversized).unwrap();
            assert!(bytes.len() <= protocol::MAX_LINE);
            let ToHost::Result {
                id,
                ok,
                result,
                error,
            } = serde_json::from_slice(&bytes).unwrap()
            else {
                panic!("expected a result");
            };
            assert_eq!(id, 42);
            assert!(!ok);
            assert!(result.is_null());
            assert_eq!(
                error.as_deref(),
                Some("reply exceeds the 256 KiB line limit")
            );
        }
        assert!(
            encode(&ToHost::State {
                store: Value::String("x".repeat(protocol::MAX_LINE))
            })
            .is_err()
        );
    }

    #[test]
    fn snapshot_budget_counts_escaping_and_keeps_identifiers_intact() {
        let large = json!({"id": "keep-me", "body": "\0🦀".repeat(100_000)});
        let small = json!({"id": "small", "body": "updated"});
        let items = bounded_array([large, small.clone()], 1024);
        assert_eq!(items, [small]);
        assert!(
            encode(&ToHost::State {
                store: json!({"items": items})
            })
            .is_ok()
        );
    }

    #[test]
    fn snapshot_budget_caps_many_entries() {
        let items = bounded_array(
            (0..1000).map(|id| json!({"id": id, "text": "\0".repeat(1000)})),
            32 * 1024,
        );
        assert!(!items.is_empty());
        assert!(serde_json::to_vec(&items).unwrap().len() <= 32 * 1024);
    }
}

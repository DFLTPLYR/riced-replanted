use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{self, BufRead, BufReader, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

use mlua::{Function, Lua, LuaSerdeExt, RegistryKey, Table, Value, Variadic};
use riced_extension_sdk::protocol::{MAX_LINE, MethodSpec, ToHost, ToPlugin, VERSION};
use serde_json::Value as Json;

const PENDING_MAX: usize = 64;
const QUEUE: usize = 16;
const POLL_MAX: usize = 256;
const SCAN_EVERY: Duration = Duration::from_secs(1);
const RESTART_DELAY: Duration = Duration::from_secs(1);
const FAILURE_LIMIT: u32 = 5;

type Pending = Arc<Mutex<HashMap<u64, RegistryKey>>>;

pub enum Update {
    Event(String, Json),
}

struct Handle {
    conn: u64,
    store: Json,
    alive: bool,
    methods: HashMap<String, MethodSpec>,
    writer: SyncSender<Vec<u8>>,
    pending: Pending,
    next: Arc<AtomicU64>,
    requested: Arc<Mutex<HashSet<String>>>,
}

enum Incoming {
    Register {
        conn: u64,
        v: u32,
        name: String,
        methods: HashMap<String, MethodSpec>,
        writer: SyncSender<Vec<u8>>,
    },
    State {
        conn: u64,
        store: Json,
    },
    Event {
        conn: u64,
        event: Json,
    },
    Result {
        conn: u64,
        id: u64,
        ok: bool,
        result: Json,
        error: Option<String>,
    },
    Closed {
        conn: u64,
    },
}

pub struct Plugins {
    socket: Option<PathBuf>,
    incoming: Receiver<Incoming>,
    conns: HashMap<u64, String>,
    handles: HashMap<String, Handle>,
    supervisor: Option<Supervisor>,
    stop: Arc<AtomicBool>,
}

impl Plugins {
    /// Opens the extension socket and supervises executable entries in the profile's
    /// `extensions/` directory. `shell` is the profile's `shell.lua` path.
    pub fn start(shell: &Path) -> io::Result<Self> {
        let profile_dir = shell.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "shell path has no parent")
        })?;
        let profile = profile_dir
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid profile name"))?
            .to_owned();
        let runtime = env::var_os("XDG_RUNTIME_DIR")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
        let socket = PathBuf::from(runtime)
            .join("riced")
            .join(format!("{profile}.sock"));
        let (incoming, stop) = listen(&socket)?;
        let dir = profile_dir.join("extensions");
        let supervisor = dir
            .is_dir()
            .then(|| Supervisor::new(dir, socket.clone(), profile.clone()));
        Ok(Self {
            socket: Some(socket),
            incoming,
            conns: HashMap::new(),
            handles: HashMap::new(),
            supervisor,
            stop,
        })
    }

    /// A registry with no socket or supervisor, used when plugins are not enabled (tests).
    #[cfg(test)]
    pub fn detached() -> Self {
        let (_tx, rx) = mpsc::channel();
        Self {
            socket: None,
            incoming: rx,
            conns: HashMap::new(),
            handles: HashMap::new(),
            supervisor: None,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    #[cfg(test)]
    fn test(incoming: Receiver<Incoming>) -> Self {
        Self {
            socket: None,
            incoming,
            conns: HashMap::new(),
            handles: HashMap::new(),
            supervisor: None,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Builds `__riced_plugins` proxies for every known extension in this Lua state.
    pub fn install(&mut self, lua: &Lua) -> mlua::Result<()> {
        let globals = lua.globals();
        if globals.get::<Option<Table>>("__riced_plugins")?.is_none() {
            globals.set("__riced_plugins", lua.create_table()?)?;
        }
        let names = self.handles.keys().cloned().collect::<Vec<_>>();
        for name in names {
            self.install_one(lua, &name)?;
        }
        Ok(())
    }

    /// Drops callbacks that belong to a Lua state that is about to be replaced.
    pub fn clear_pending(&mut self) {
        for handle in self.handles.values_mut() {
            handle.pending.lock().unwrap().clear();
        }
    }

    /// Applies extension messages. Returns whether anything should be redrawn, plus events
    /// that the caller must feed into `app:update`.
    pub fn poll(&mut self, lua: &Lua) -> mlua::Result<(bool, Vec<Update>)> {
        if let Some(supervisor) = &mut self.supervisor {
            supervisor.poll();
        }
        let mut changed = false;
        let mut events = Vec::new();
        for _ in 0..POLL_MAX {
            let message = match self.incoming.try_recv() {
                Ok(message) => message,
                Err(_) => break,
            };
            match message {
                Incoming::Register {
                    conn,
                    v,
                    name,
                    methods,
                    writer,
                } => {
                    if v != VERSION {
                        reject(&writer, &format!("unsupported protocol version {v}"));
                        continue;
                    }
                    if !valid_name(&name) {
                        reject(&writer, "name must match [a-z0-9_-]{1,32}");
                        continue;
                    }
                    if self.handles.get(&name).is_some_and(|handle| handle.alive) {
                        reject(&writer, &format!("duplicate plugin {name}"));
                        continue;
                    }
                    if let Some(old) = self.handles.remove(&name) {
                        self.conns.remove(&old.conn);
                    }
                    changed = true;
                    self.conns.insert(conn, name.clone());
                    self.handles.insert(
                        name.clone(),
                        Handle {
                            conn,
                            store: Json::Null,
                            alive: true,
                            methods,
                            writer,
                            pending: Arc::new(Mutex::new(HashMap::new())),
                            next: Arc::new(AtomicU64::new(1)),
                            requested: Arc::new(Mutex::new(HashSet::new())),
                        },
                    );
                    self.install_one(lua, &name)?;
                    self.refresh_store(lua, &name)?;
                    self.set_alive(lua, &name, true)?;
                }
                Incoming::State { conn, store } => {
                    let Some(name) = self.conns.get(&conn).cloned() else {
                        continue;
                    };
                    let Some(handle) = self.handles.get_mut(&name) else {
                        continue;
                    };
                    handle.store = store;
                    changed = true;
                    self.refresh_store(lua, &name)?;
                }
                Incoming::Event { conn, event } => {
                    if let Some(name) = self.conns.get(&conn) {
                        events.push(Update::Event(name.clone(), event));
                    }
                }
                Incoming::Result {
                    conn,
                    id,
                    ok,
                    result,
                    error,
                } => {
                    let Some(name) = self.conns.get(&conn).cloned() else {
                        continue;
                    };
                    let key = self
                        .handles
                        .get_mut(&name)
                        .and_then(|handle| handle.pending.lock().unwrap().remove(&id));
                    if let Some(key) = key {
                        let callback: Function = lua.registry_value(&key)?;
                        let value = lua.to_value(&result)?;
                        let error = match error {
                            Some(error) => Value::String(lua.create_string(error)?),
                            None => Value::Nil,
                        };
                        if ok {
                            callback.call::<()>((value, Value::Nil))?;
                        } else {
                            callback.call::<()>((Value::Nil, error))?;
                        }
                        lua.remove_registry_value(key)?;
                        changed = true;
                    }
                }
                Incoming::Closed { conn } => {
                    let Some(name) = self.conns.remove(&conn) else {
                        continue;
                    };
                    let pending = {
                        let Some(handle) = self.handles.get_mut(&name) else {
                            continue;
                        };
                        if handle.conn != conn {
                            continue;
                        }
                        handle.alive = false;
                        handle.pending.lock().unwrap().drain().collect::<Vec<_>>()
                    };
                    for (_, key) in pending {
                        let callback: Function = lua.registry_value(&key)?;
                        callback.call::<()>((
                            Value::Nil,
                            Value::String(lua.create_string("plugin disconnected")?),
                        ))?;
                        lua.remove_registry_value(key)?;
                    }
                    self.set_alive(lua, &name, false)?;
                    changed = true;
                }
            }
        }
        Ok((changed, events))
    }

    fn install_one(&self, lua: &Lua, name: &str) -> mlua::Result<()> {
        let Some(handle) = self.handles.get(name) else {
            return Ok(());
        };
        let plugins: Table = lua.globals().get("__riced_plugins")?;
        // Reuse an existing proxy so a value captured before registration stays live.
        let proxy = match plugins.get::<Option<Table>>(name)? {
            Some(proxy) => proxy,
            None => {
                let proxy = lua.create_table()?;
                plugins.set(name, proxy.clone())?;
                proxy
            }
        };
        proxy.set("name", name)?;
        proxy.set("alive", handle.alive)?;
        if let Some(meta) = lua.globals().get::<Option<Table>>("__riced_plugin_meta")? {
            proxy.set_metatable(Some(meta))?;
        }
        let store = match proxy.get::<Option<Table>>("store")? {
            Some(store) => store,
            None => {
                let store = lua.create_table()?;
                proxy.set("store", store.clone())?;
                store
            }
        };
        fill(lua, &store, &handle.store)?;

        // Reading a missing store field subscribes to it; extensions only compute what
        // something actually asks for. The metatable survives store refills.
        let requested = Arc::clone(&handle.requested);
        let writer = handle.writer.clone();
        let index = lua.create_function(move |_, (_store, key): (Table, Value)| {
            let Value::String(key) = key else {
                return Ok(Value::Nil);
            };
            let key = key.to_str()?.to_owned();
            if key.is_empty() || key.starts_with("__") {
                return Ok(Value::Nil);
            }
            let added = requested.lock().unwrap().insert(key.clone());
            if added {
                send(&writer, &ToPlugin::Subscribe { fields: vec![key] });
            }
            Ok(Value::Nil)
        })?;
        let meta = lua.create_table()?;
        meta.set("__index", index)?;
        store.set_metatable(Some(meta))?;

        for method in handle.methods.keys() {
            let writer = handle.writer.clone();
            let pending = handle.pending.clone();
            let next = handle.next.clone();
            let key = method.clone();
            let method = method.clone();
            let plugin = name.to_owned();
            let function = lua.create_function(move |lua, values: Variadic<Value>| {
                call(lua, &plugin, &method, &writer, &pending, &next, values)
            })?;
            proxy.set(key.as_str(), function)?;
        }
        Ok(())
    }

    fn refresh_store(&self, lua: &Lua, name: &str) -> mlua::Result<()> {
        let Some(handle) = self.handles.get(name) else {
            return Ok(());
        };
        let plugins: Table = lua.globals().get("__riced_plugins")?;
        let Some(proxy) = plugins.get::<Option<Table>>(name)? else {
            return Ok(());
        };
        fill(lua, &proxy.get::<Table>("store")?, &handle.store)
    }

    fn set_alive(&self, lua: &Lua, name: &str, alive: bool) -> mlua::Result<()> {
        let plugins: Table = lua.globals().get("__riced_plugins")?;
        if let Some(proxy) = plugins.get::<Option<Table>>(name)? {
            proxy.set("alive", alive)?;
        }
        Ok(())
    }
}

impl Drop for Plugins {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(supervisor) = &mut self.supervisor {
            supervisor.stop();
        }
        if let Some(socket) = &self.socket {
            let _ = fs::remove_file(socket);
        }
    }
}

fn call(
    lua: &Lua,
    plugin: &str,
    method: &str,
    writer: &SyncSender<Vec<u8>>,
    pending: &Pending,
    next: &Arc<AtomicU64>,
    mut values: Variadic<Value>,
) -> mlua::Result<()> {
    let callback = match values.last() {
        Some(Value::Function(_)) => values.pop(),
        _ => None,
    };
    let id = next.fetch_add(1, Ordering::Relaxed);
    if let Some(callback) = callback {
        let mut pending = pending.lock().unwrap();
        if pending.len() >= PENDING_MAX {
            return Err(mlua::Error::runtime(format!(
                "plugin {plugin} has too many pending calls"
            )));
        }
        pending.insert(id, lua.create_registry_value(callback)?);
    }
    let args = values
        .iter()
        .cloned()
        .map(|value| lua.from_value::<Json>(value))
        .collect::<mlua::Result<Vec<_>>>()?;
    let message = ToPlugin::Call {
        id,
        method: method.to_owned(),
        args,
    };
    let mut line =
        serde_json::to_vec(&message).map_err(|error| mlua::Error::runtime(error.to_string()))?;
    line.push(b'\n');
    match writer.try_send(line) {
        Ok(()) | Err(TrySendError::Disconnected(_)) => Ok(()),
        Err(TrySendError::Full(_)) => {
            pending.lock().unwrap().remove(&id);
            Err(mlua::Error::runtime(format!("plugin {plugin} is busy")))
        }
    }
}

fn fill(lua: &Lua, table: &Table, json: &Json) -> mlua::Result<()> {
    table.clear()?;
    if json.is_null() {
        return Ok(());
    }
    if let Value::Table(source) = lua.to_value(json)? {
        for pair in source.pairs::<Value, Value>() {
            let (key, value) = pair?;
            table.raw_set(key, value)?;
        }
    }
    Ok(())
}

fn send(writer: &SyncSender<Vec<u8>>, message: &ToPlugin) {
    let Ok(mut line) = serde_json::to_vec(message) else {
        return;
    };
    line.push(b'\n');
    let _ = writer.try_send(line);
}

fn reject(writer: &SyncSender<Vec<u8>>, error: &str) {
    send(
        writer,
        &ToPlugin::Error {
            error: error.to_owned(),
        },
    );
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
}

fn listen(socket: &Path) -> io::Result<(Receiver<Incoming>, Arc<AtomicBool>)> {
    if let Some(parent) = socket.parent() {
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    if UnixStream::connect(socket).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("{} is already in use", socket.display()),
        ));
    }
    match fs::remove_file(socket) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let (tx, rx) = mpsc::sync_channel(QUEUE);
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    thread::Builder::new()
        .name("riced-plugins".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            let mut next = 1u64;
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let id = next;
                        next += 1;
                        let tx = tx.clone();
                        let stop = thread_stop.clone();
                        let _ = thread::Builder::new()
                            .name(format!("riced-plugin-{id}"))
                            .stack_size(256 * 1024)
                            .spawn(move || read_conn(stream, id, tx, stop));
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
        })?;
    Ok((rx, stop))
}

fn read_conn(stream: UnixStream, id: u64, tx: SyncSender<Incoming>, stop: Arc<AtomicBool>) {
    let mut reader = BufReader::new(stream);
    let writer = match reader.get_ref().try_clone() {
        Ok(writer) => writer,
        Err(_) => return,
    };
    let (out_tx, out_rx) = mpsc::sync_channel::<Vec<u8>>(QUEUE);
    if thread::Builder::new()
        .name(format!("riced-plugin-{id}-writer"))
        .stack_size(128 * 1024)
        .spawn(move || write_conn(writer, out_rx))
        .is_err()
    {
        return;
    }
    let mut line = Vec::new();
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) if line.len() > MAX_LINE => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let message = match serde_json::from_slice::<ToHost>(&line) {
            Ok(message) => message,
            Err(error) => {
                eprintln!("riced: bad extension message: {error}");
                continue;
            }
        };
        let message = match message {
            ToHost::Register {
                v,
                name,
                methods,
                pid: _,
                ..
            } => Incoming::Register {
                conn: id,
                v,
                name,
                methods,
                writer: out_tx.clone(),
            },
            ToHost::State { store } => Incoming::State { conn: id, store },
            ToHost::Event { event } => Incoming::Event { conn: id, event },
            ToHost::Result {
                id: call_id,
                ok,
                result,
                error,
            } => Incoming::Result {
                conn: id,
                id: call_id,
                ok,
                result,
                error,
            },
        };
        if tx.send(message).is_err() {
            break;
        }
    }
    let _ = tx.send(Incoming::Closed { conn: id });
}

fn write_conn(mut stream: UnixStream, messages: Receiver<Vec<u8>>) {
    for message in messages {
        if stream.write_all(&message).is_err() {
            break;
        }
    }
}

struct Supervisor {
    dir: PathBuf,
    socket: PathBuf,
    profile: String,
    children: HashMap<PathBuf, ChildEntry>,
    last_scan: Instant,
}

struct ChildEntry {
    child: Option<Child>,
    failures: u32,
    started: Instant,
    restart_at: Option<Instant>,
}

impl Supervisor {
    fn new(dir: PathBuf, socket: PathBuf, profile: String) -> Self {
        Self {
            dir,
            socket,
            profile,
            children: HashMap::new(),
            last_scan: Instant::now() - SCAN_EVERY,
        }
    }

    fn poll(&mut self) {
        let now = Instant::now();
        let mut removed = Vec::new();
        let mut restart = Vec::new();
        for (path, entry) in &mut self.children {
            if !path.exists() {
                if let Some(mut child) = entry.child.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                removed.push(path.clone());
                continue;
            }
            let exited = match entry.child.as_mut() {
                Some(child) => match child.try_wait() {
                    Ok(Some(status)) => {
                        eprintln!("riced: extension {} exited ({status})", path.display());
                        entry.child = None;
                        true
                    }
                    Ok(None) => false,
                    Err(error) => {
                        eprintln!("riced: cannot poll extension {}: {error}", path.display());
                        entry.child = None;
                        true
                    }
                },
                None => false,
            };
            if exited {
                entry.failures = if entry.started.elapsed() < RESTART_DELAY * 2 {
                    entry.failures + 1
                } else {
                    0
                };
                if entry.failures < FAILURE_LIMIT {
                    entry.restart_at = Some(now + RESTART_DELAY);
                } else {
                    entry.restart_at = None;
                    eprintln!(
                        "riced: extension {} keeps crashing; giving up",
                        path.display()
                    );
                }
            }
            if entry.child.is_none() && entry.restart_at.is_some_and(|at| now >= at) {
                restart.push(path.clone());
            }
        }
        for path in removed {
            self.children.remove(&path);
        }
        for path in restart {
            self.spawn(&path);
        }
        if now.duration_since(self.last_scan) >= SCAN_EVERY {
            self.last_scan = now;
            let new = scan(&self.dir)
                .into_iter()
                .filter(|path| !self.children.contains_key(path))
                .collect::<Vec<_>>();
            for path in new {
                self.children.insert(
                    path.clone(),
                    ChildEntry {
                        child: None,
                        failures: 0,
                        started: now,
                        restart_at: None,
                    },
                );
                self.spawn(&path);
            }
        }
    }

    fn spawn(&mut self, path: &Path) {
        let Some(entry) = self.children.get_mut(path) else {
            return;
        };
        let result = Command::new(path)
            .current_dir(path.parent().unwrap_or(Path::new(".")))
            .env("RICED_SOCKET", &self.socket)
            .env("RICED_PROFILE", &self.profile)
            .spawn();
        match result {
            Ok(child) => {
                eprintln!("riced: started extension {}", path.display());
                entry.child = Some(child);
                entry.started = Instant::now();
                entry.restart_at = None;
            }
            Err(error) => {
                entry.failures += 1;
                entry.restart_at = Some(Instant::now() + RESTART_DELAY);
                eprintln!("riced: cannot start extension {}: {error}", path.display());
            }
        }
    }

    fn stop(&mut self) {
        for entry in self.children.values_mut() {
            if let Some(mut child) = entry.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        self.children.clear();
    }
}

fn scan(dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return paths;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if executable(&path) {
            paths.push(path);
        } else if path.is_dir()
            && let Ok(inner) = fs::read_dir(&path)
        {
            for entry in inner.flatten() {
                let path = entry.path();
                if executable(&path) {
                    paths.push(path);
                }
            }
        }
    }
    paths.sort();
    paths
}

fn executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (SyncSender<Incoming>, Plugins, Lua) {
        let (tx, rx) = mpsc::sync_channel(16);
        let mut plugins = Plugins::test(rx);
        let lua = Lua::new();
        crate::lua::load(&lua).unwrap();
        plugins.install(&lua).unwrap();
        (tx, plugins, lua)
    }

    fn register(tx: &SyncSender<Incoming>, methods: &[&str]) -> Receiver<Vec<u8>> {
        let (writer, calls) = mpsc::sync_channel(16);
        let methods = methods
            .iter()
            .map(|name| ((*name).to_owned(), MethodSpec::default()))
            .collect();
        tx.send(Incoming::Register {
            conn: 1,
            v: VERSION,
            name: "demo".to_owned(),
            methods,
            writer,
        })
        .unwrap();
        calls
    }

    #[test]
    fn publishes_state_and_preserves_table_identity() {
        let (tx, mut plugins, lua) = setup();
        let _calls = register(&tx, &[]);
        plugins.poll(&lua).unwrap();
        let first: Table = lua
            .globals()
            .get::<Table>("__riced_plugins")
            .unwrap()
            .get("demo")
            .unwrap();

        tx.send(Incoming::State {
            conn: 1,
            store: serde_json::json!({"value": 2}),
        })
        .unwrap();
        let (changed, _) = plugins.poll(&lua).unwrap();

        assert!(changed);
        assert_eq!(
            first
                .get::<Table>("store")
                .unwrap()
                .get::<u32>("value")
                .unwrap(),
            2
        );
        assert_eq!(
            lua.load("return riced.plugin('demo').value")
                .eval::<u32>()
                .unwrap(),
            2
        );
    }

    #[test]
    fn method_call_reaches_the_extension_with_arguments() {
        let (tx, mut plugins, lua) = setup();
        let calls = register(&tx, &["focus"]);
        plugins.poll(&lua).unwrap();
        lua.load("riced.plugins('demo').focus('two', 3)")
            .exec()
            .unwrap();

        let line = calls.recv_timeout(Duration::from_secs(1)).unwrap();
        let message: ToPlugin = serde_json::from_slice(&line).unwrap();
        match message {
            ToPlugin::Call { method, args, .. } => {
                assert_eq!(method, "focus");
                assert_eq!(args, vec![serde_json::json!("two"), serde_json::json!(3)]);
            }
            _ => panic!("expected call"),
        }
    }

    #[test]
    fn reading_a_store_field_subscribes_once() {
        let (tx, mut plugins, lua) = setup();
        let calls = register(&tx, &[]);
        plugins.poll(&lua).unwrap();

        lua.load("return riced.plugins('demo').store.cpu")
            .exec()
            .unwrap();
        let line = calls.recv_timeout(Duration::from_secs(1)).unwrap();
        match serde_json::from_slice::<ToPlugin>(&line).unwrap() {
            ToPlugin::Subscribe { fields } => assert_eq!(fields, ["cpu"]),
            _ => panic!("expected subscribe"),
        }

        lua.load("return riced.plugins('demo').store.cpu")
            .exec()
            .unwrap();
        assert!(calls.try_recv().is_err());
    }

    #[test]
    fn result_invokes_the_lua_callback() {
        let (tx, mut plugins, lua) = setup();
        let _calls = register(&tx, &["query"]);
        plugins.poll(&lua).unwrap();
        tx.send(Incoming::Result {
            conn: 1,
            id: 1,
            ok: true,
            result: serde_json::json!({"answer": 42}),
            error: None,
        })
        .ok();
        lua.load(
            "answer = nil; riced.plugins('demo').query(function(result) answer = result.answer end)",
        )
        .exec()
        .unwrap();
        plugins.poll(&lua).unwrap();

        assert_eq!(lua.globals().get::<u32>("answer").unwrap(), 42);
    }

    #[test]
    fn disconnect_keeps_the_store_and_marks_the_proxy_dead() {
        let (tx, mut plugins, lua) = setup();
        let _calls = register(&tx, &[]);
        tx.send(Incoming::State {
            conn: 1,
            store: serde_json::json!({"value": 1}),
        })
        .unwrap();
        plugins.poll(&lua).unwrap();
        tx.send(Incoming::Closed { conn: 1 }).unwrap();
        plugins.poll(&lua).unwrap();

        let proxy: Table = lua
            .globals()
            .get::<Table>("__riced_plugins")
            .unwrap()
            .get("demo")
            .unwrap();
        assert!(!proxy.get::<bool>("alive").unwrap());
        assert_eq!(
            proxy
                .get::<Table>("store")
                .unwrap()
                .get::<u32>("value")
                .unwrap(),
            1
        );
    }

    #[test]
    fn rejects_duplicate_live_names() {
        let (tx, mut plugins, lua) = setup();
        let _calls = register(&tx, &[]);
        plugins.poll(&lua).unwrap();
        let (writer, _) = mpsc::sync_channel(16);
        tx.send(Incoming::Register {
            conn: 2,
            v: VERSION,
            name: "demo".to_owned(),
            methods: HashMap::new(),
            writer,
        })
        .unwrap();

        let (changed, _) = plugins.poll(&lua).unwrap();
        assert!(!changed);
        assert_eq!(plugins.handles.len(), 1);
    }

    #[test]
    fn reload_rebuilds_proxies_from_host_snapshots() {
        let (tx, mut plugins, lua) = setup();
        let _calls = register(&tx, &[]);
        tx.send(Incoming::State {
            conn: 1,
            store: serde_json::json!({"value": 7}),
        })
        .unwrap();
        plugins.poll(&lua).unwrap();
        plugins.clear_pending();

        let next = Lua::new();
        crate::lua::load(&next).unwrap();
        plugins.install(&next).unwrap();

        let proxy: Table = next
            .globals()
            .get::<Table>("__riced_plugins")
            .unwrap()
            .get("demo")
            .unwrap();
        assert_eq!(
            proxy
                .get::<Table>("store")
                .unwrap()
                .get::<u32>("value")
                .unwrap(),
            7
        );
    }

    #[test]
    fn socket_delivers_registration_and_state() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "riced-plugins-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let socket = root.join("test.sock");
        let (incoming, _stop) = listen(&socket).unwrap();
        let mut plugins = Plugins::test(incoming);
        let lua = Lua::new();
        crate::lua::load(&lua).unwrap();
        plugins.install(&lua).unwrap();

        let mut stream = UnixStream::connect(&socket).unwrap();
        stream
            .write_all(
                b"{\"type\":\"register\",\"v\":1,\"name\":\"demo\"}\n\
                  {\"type\":\"state\",\"store\":{\"value\":9}}\n",
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let (changed, _) = plugins.poll(&lua).unwrap();
            if changed {
                break;
            }
            assert!(Instant::now() < deadline, "extension state did not arrive");
            thread::sleep(Duration::from_millis(20));
        }
        let proxy: Table = lua
            .globals()
            .get::<Table>("__riced_plugins")
            .unwrap()
            .get("demo")
            .unwrap();
        assert_eq!(
            proxy
                .get::<Table>("store")
                .unwrap()
                .get::<u32>("value")
                .unwrap(),
            9
        );

        drop(stream);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            plugins.poll(&lua).unwrap();
            if !proxy.get::<bool>("alive").unwrap() {
                break;
            }
            assert!(Instant::now() < deadline, "disconnect was not observed");
            thread::sleep(Duration::from_millis(20));
        }

        _stop.store(true, Ordering::Relaxed);
        drop(plugins);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn proxy_captured_before_registration_stays_live() {
        let (tx, mut plugins, lua) = setup();
        let early: Table = lua.load("return riced.plugins('demo')").eval().unwrap();
        assert!(!early.get::<bool>("alive").unwrap());

        let _calls = register(&tx, &[]);
        tx.send(Incoming::State {
            conn: 1,
            store: serde_json::json!({"value": 5}),
        })
        .unwrap();
        plugins.poll(&lua).unwrap();

        assert!(early.get::<bool>("alive").unwrap());
        assert_eq!(
            early
                .get::<Table>("store")
                .unwrap()
                .get::<u32>("value")
                .unwrap(),
            5
        );
    }

    #[test]
    fn supervisor_spawns_and_removes_extensions() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "riced-supervisor-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let script = root.join("demo");
        fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let mut supervisor = Supervisor::new(root.clone(), root.join("socket"), "test".to_owned());

        supervisor.poll();
        assert!(supervisor.children.get(&script).unwrap().child.is_some());

        fs::remove_file(&script).unwrap();
        supervisor.poll();
        assert!(!supervisor.children.contains_key(&script));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_invalid_names() {
        for name in ["", "Demo", "a/b", "a.b", &"x".repeat(33)] {
            assert!(!valid_name(name), "{name}");
        }
        assert!(valid_name("system"));
        assert!(valid_name("my-plugin_2"));
    }
}

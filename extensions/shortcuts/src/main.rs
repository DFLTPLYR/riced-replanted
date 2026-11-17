//! Compositor-global keybindings that emit riced Lua events.
//!
//! `bind(keys, label)` registers a keybinding whose `exec` appends the label
//! to a queue file under `$XDG_RUNTIME_DIR/riced/`; a poll loop drains that
//! queue every 200 ms and forwards one `{"shortcut": label}` plugin event per
//! line, so key presses arrive in Lua as ordinary `app:update` plugin events.
//! The extension itself never runs a shell: the shell command is executed by
//! the compositor, this process only talks to compositor IPC sockets.
//!
//! `keys` is backend-native and passes through verbatim: Hyprland wants
//! `"SUPER,Return"`, sway wants `"Super+Return"`. Every bind first issues the
//! backend's unbind for the same keys because the compositor remembers binds
//! across extension restarts; unbind-then-bind keeps re-binding idempotent.
//! Backend detection: `$HYPRLAND_INSTANCE_SIGNATURE` selects Hyprland's
//! `.socket2.sock`, else `$SWAYSOCK` selects sway's i3-ipc socket; with
//! neither, the store reports `connected = false` and methods fail.
//! Labels are validated against `[A-Za-z0-9_.-]{1,64}` because they end up
//! inside the shell command, as does the queue path, which is quoted and
//! rejected when it contains a single quote.

use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, Read, Write},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use riced_extension_sdk::{Plugin, log};
use serde_json::{Value, json};

const POLL: Duration = Duration::from_millis(200);
/// Queue lines come from `printf %s\n label`; anything longer is garbage.
const MAX_LINE: usize = 256;
/// A queue that grew past this between polls is dropped instead of parsed.
const MAX_QUEUE: u64 = 64 * 1024;
const BACKEND_TIMEOUT: Duration = Duration::from_secs(2);
const REPLY_LIMIT: u64 = 64 * 1024;
/// i3-ipc frame: magic, u32 LE payload length, u32 LE message type.
const I3_MAGIC: &[u8; 6] = b"i3-ipc";
const I3_HEADER: usize = I3_MAGIC.len() + 8;
const RUN_COMMAND: u32 = 0;
const NOT_CONNECTED: &str = "no compositor detected: set HYPRLAND_INSTANCE_SIGNATURE or SWAYSOCK";

enum Backend {
    Hyprland { socket: PathBuf },
    Sway { socket: PathBuf },
}

impl Backend {
    fn name(&self) -> &'static str {
        match self {
            Backend::Hyprland { .. } => "hyprland",
            Backend::Sway { .. } => "sway",
        }
    }

    fn socket(&self) -> &Path {
        match self {
            Backend::Hyprland { socket } | Backend::Sway { socket } => socket,
        }
    }

    /// Issues one backend command; Ok only when the compositor confirms it.
    fn command(&self, command: &str) -> Result<(), String> {
        match self {
            Backend::Hyprland { socket } => hyprland_command(socket, command),
            Backend::Sway { socket } => sway_command(socket, command),
        }
    }

    fn bind(&self, keys: &str, command: &str) -> Result<(), String> {
        self.command(&match self {
            Backend::Hyprland { .. } => format!("keyword bind = {keys},exec,{command}"),
            Backend::Sway { .. } => format!("bindsym {keys} exec {command}"),
        })
    }

    fn unbind(&self, keys: &str) -> Result<(), String> {
        self.command(&match self {
            Backend::Hyprland { .. } => format!("keyword unbind {keys}"),
            Backend::Sway { .. } => format!("unbindsym {keys}"),
        })
    }
}

/// Hyprland wins when both environments are present.
fn detect_backend() -> Option<Backend> {
    if let Some(signature) = env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .ok()
        .filter(|signature| !signature.is_empty())
    {
        let runtime = env::var("XDG_RUNTIME_DIR")
            .ok()
            .filter(|dir| !dir.is_empty())?;
        return Some(Backend::Hyprland {
            socket: PathBuf::from(runtime)
                .join("hypr")
                .join(signature)
                .join(".socket2.sock"),
        });
    }
    env::var("SWAYSOCK")
        .ok()
        .filter(|socket| !socket.is_empty())
        .map(|socket| Backend::Sway {
            socket: PathBuf::from(socket),
        })
}

/// Hyprland's socket2 speaks newline-terminated text commands and answers
/// `ok` when a command was accepted.
fn hyprland_command(socket: &Path, command: &str) -> Result<(), String> {
    let mut stream =
        UnixStream::connect(socket).map_err(|error| format!("hyprland connect failed: {error}"))?;
    stream
        .set_read_timeout(Some(BACKEND_TIMEOUT))
        .map_err(|error| format!("hyprland socket: {error}"))?;
    stream
        .write_all(format!("{command}\n").as_bytes())
        .map_err(|error| format!("hyprland write failed: {error}"))?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|error| format!("hyprland socket: {error}"))?;
    let mut response = String::new();
    (&mut stream)
        .take(REPLY_LIMIT)
        .read_to_string(&mut response)
        .map_err(|error| format!("hyprland reply failed: {error}"))?;
    match response.trim() {
        "ok" => Ok(()),
        reply => Err(format!("hyprland rejected '{command}': {reply}")),
    }
}

/// sway's socket speaks i3-ipc: framed JSON, one connection per command.
fn sway_command(socket: &Path, command: &str) -> Result<(), String> {
    let mut stream =
        UnixStream::connect(socket).map_err(|error| format!("sway connect failed: {error}"))?;
    stream
        .set_read_timeout(Some(BACKEND_TIMEOUT))
        .map_err(|error| format!("sway socket: {error}"))?;
    stream
        .write_all(&i3_frame(RUN_COMMAND, command))
        .map_err(|error| format!("sway write failed: {error}"))?;
    let mut header = [0u8; I3_HEADER];
    stream
        .read_exact(&mut header)
        .map_err(|error| format!("sway reply failed: {error}"))?;
    if &header[..I3_MAGIC.len()] != I3_MAGIC {
        return Err("sway sent a reply without the i3-ipc magic".into());
    }
    let length = u32::from_le_bytes(header[6..10].try_into().unwrap()) as usize;
    if length as u64 > REPLY_LIMIT {
        return Err(format!(
            "sway reply of {length} bytes exceeds the {REPLY_LIMIT}-byte limit"
        ));
    }
    let mut payload = vec![0u8; length];
    stream
        .read_exact(&mut payload)
        .map_err(|error| format!("sway reply failed: {error}"))?;
    i3_parse(&payload).map_err(|error| format!("sway rejected '{command}': {error}"))
}

fn i3_frame(message_type: u32, payload: &str) -> Vec<u8> {
    let mut frame = Vec::with_capacity(I3_HEADER + payload.len());
    frame.extend_from_slice(I3_MAGIC);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&message_type.to_le_bytes());
    frame.extend_from_slice(payload.as_bytes());
    frame
}

/// A RUN_COMMAND reply is `[{"success":bool,"error":...}, ...]`; every entry
/// must report success, and the first failure carries the error text.
fn i3_parse(payload: &[u8]) -> Result<(), String> {
    let replies: Value = serde_json::from_slice(payload)
        .map_err(|error| format!("malformed sway reply: {error}"))?;
    let replies = replies
        .as_array()
        .filter(|replies| !replies.is_empty())
        .ok_or("malformed sway reply: expected a non-empty array")?;
    for reply in replies {
        if reply.get("success").and_then(Value::as_bool) != Some(true) {
            let error = reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unspecified failure");
            return Err(error.to_owned());
        }
    }
    Ok(())
}

/// `[A-Za-z0-9_.-]{1,64}` — the label is embedded verbatim in the shell
/// command the compositor execs.
fn validate_label(label: &str) -> Result<(), String> {
    if !label.is_empty()
        && label.len() <= 64
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    {
        return Ok(());
    }
    Err(format!(
        "label must match [A-Za-z0-9_.-]{{1,64}}, got {label:?}"
    ))
}

/// Keys pass through verbatim into compositor command syntax, so control
/// characters and shell or command separators are rejected; spaces are fine
/// (Hyprland writes `SUPER SHIFT,Return`).
fn validate_keys(keys: &str) -> Result<(), String> {
    if keys.is_empty() || keys.len() > 128 {
        return Err(format!("keys must be 1..=128 bytes, got {}", keys.len()));
    }
    if keys.bytes().any(|byte| {
        byte.is_ascii_control() || matches!(byte, b'\'' | b'"' | b'`' | b'\\' | b';' | b'$')
    }) {
        return Err(format!(
            "keys must be printable ASCII without quote, backslash, semicolon or dollar, got {keys:?}"
        ));
    }
    Ok(())
}

/// `$XDG_RUNTIME_DIR/riced/shortcuts-<profile>.queue`; `<profile>` is the
/// file stem of `RICED_SOCKET`. Both end up inside the exec'd shell command,
/// so a single quote in either would break the quoting.
fn queue_path() -> Result<PathBuf, String> {
    let runtime = env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|dir| !dir.is_empty())
        .ok_or("XDG_RUNTIME_DIR is not set")?;
    let socket = env::var("RICED_SOCKET")
        .ok()
        .filter(|path| !path.is_empty())
        .ok_or("RICED_SOCKET is not set")?;
    let id = Path::new(&socket)
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .filter(|stem| !stem.is_empty())
        .ok_or("RICED_SOCKET has no usable file stem")?;
    validate_label(id)
        .map_err(|_| "RICED_SOCKET file stem must match [A-Za-z0-9_.-]{1,64}".to_owned())?;
    if runtime.contains('\'') {
        return Err("XDG_RUNTIME_DIR must not contain a single quote".into());
    }
    Ok(Path::new(&runtime)
        .join("riced")
        .join(format!("shortcuts-{id}.queue")))
}

/// Resolves the queue path and creates its 0700 directory; None disables
/// event forwarding (binds still fail with the precise error).
fn prepare_queue() -> Option<PathBuf> {
    let queue = match queue_path() {
        Ok(queue) => queue,
        Err(error) => {
            log::warn!(target: "shortcuts.queue", "shortcut events disabled: {error}");
            return None;
        }
    };
    let parent = queue.parent().unwrap_or(Path::new("."));
    if let Err(error) = fs::create_dir_all(parent)
        .and_then(|()| fs::set_permissions(parent, fs::Permissions::from_mode(0o700)))
    {
        log::warn!(target: "shortcuts.queue", "shortcut events disabled: {error}");
        return None;
    }
    Some(queue)
}

/// What the compositor execs per key press. The `\\n` reaches printf as the
/// two characters `\n`, so printf itself writes the newline; the label is
/// validated and the queue path is double-quoted inside the outer quotes.
fn exec_command(label: &str, queue: &Path) -> String {
    format!("sh -c 'printf %s\\n {label} >> {queue:?}'")
}

/// Empties the queue: reads it, truncates it, returns the labels to forward.
fn drain_queue(path: &Path) -> io::Result<Vec<String>> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    if file.metadata()?.len() > MAX_QUEUE {
        log::warn!(target: "shortcuts.queue", "queue exceeded {MAX_QUEUE} bytes; dropping its contents");
        file.set_len(0)?;
        return Ok(Vec::new());
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    file.set_len(0)?;
    Ok(parse_queue(&String::from_utf8_lossy(&bytes)))
}

/// One label per line; empty and over-long lines are ignored.
fn parse_queue(contents: &str) -> Vec<String> {
    contents
        .lines()
        .filter(|line| {
            if line.is_empty() {
                return false;
            }
            if line.len() > MAX_LINE {
                log::warn!(target: "shortcuts.queue", "ignoring a {}-byte queue line", line.len());
                return false;
            }
            true
        })
        .map(str::to_owned)
        .collect()
}

fn snapshot(connected: bool, compositor: Option<&str>, binds: &BTreeMap<String, String>) -> Value {
    json!({
        "connected": connected,
        "compositor": compositor,
        "binds": binds
            .iter()
            .map(|(keys, label)| json!({"keys": keys, "label": label}))
            .collect::<Vec<_>>(),
    })
}

fn str_arg<'a>(args: &'a Value, index: usize, what: &str) -> Result<&'a str, String> {
    args.get(index)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {what} string at position {index}"))
}

fn main() -> io::Result<()> {
    let backend = detect_backend().map(Arc::new);
    let binds = Arc::new(Mutex::new(BTreeMap::<String, String>::new()));

    let bind_backend = backend.clone();
    let unbind_backend = backend.clone();
    let bind_binds = Arc::clone(&binds);
    let unbind_binds = Arc::clone(&binds);

    Plugin::new("shortcuts")
        .state(["connected", "compositor", "binds"])
        .method("bind", false, move |args| {
            let keys = str_arg(args, 0, "keys")?;
            let label = str_arg(args, 1, "label")?;
            validate_keys(keys)?;
            validate_label(label)?;
            let backend = bind_backend.as_ref().ok_or(NOT_CONNECTED)?;
            let command = exec_command(label, &queue_path()?);
            backend
                .unbind(keys)
                .and_then(|()| backend.bind(keys, &command))
                .inspect_err(
                    |error| log::warn!(target: "shortcuts.bind", "bind {keys}: {error}"),
                )?;
            bind_binds
                .lock()
                .map_err(|_| "binds lock poisoned".to_owned())?
                .insert(keys.to_owned(), label.to_owned());
            Ok(Value::Null)
        })
        .method("unbind", false, move |args| {
            let keys = str_arg(args, 0, "keys")?;
            validate_keys(keys)?;
            let backend = unbind_backend.as_ref().ok_or(NOT_CONNECTED)?;
            let result = backend.unbind(keys).inspect_err(
                |error| log::warn!(target: "shortcuts.bind", "unbind {keys}: {error}"),
            );
            // The intent is unbound either way; keep the store consistent.
            if let Ok(mut binds) = unbind_binds.lock() {
                binds.remove(keys);
            }
            result?;
            Ok(Value::Null)
        })
        .serve(move |context| {
            match backend.as_ref() {
                Some(backend) => log::info!(
                    target: "shortcuts.backend",
                    "using the {} backend at {}",
                    backend.name(),
                    backend.socket().display()
                ),
                None => log::warn!(target: "shortcuts.backend", "{NOT_CONNECTED}"),
            }
            let queue = prepare_queue();
            let mut last = Value::Null;
            loop {
                let Ok(binds) = binds.lock() else {
                    log::error!(target: "shortcuts.bind", "binds lock poisoned");
                    return;
                };
                let snapshot = snapshot(
                    backend.is_some(),
                    backend.as_deref().map(Backend::name),
                    &binds,
                );
                drop(binds);
                if snapshot != last {
                    context.publish(snapshot.clone());
                    last = snapshot;
                }
                if let Some(queue) = &queue {
                    match drain_queue(queue) {
                        Ok(labels) => {
                            for label in labels {
                                context.event(json!({ "shortcut": label }));
                            }
                        }
                        Err(error) => log::warn!(target: "shortcuts.queue", "{error}"),
                    }
                }
                thread::sleep(POLL);
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_reject_empty_long_and_shell_metacharacters() {
        let long = "x".repeat(65);
        for label in [
            "",
            "a b",
            "launch;rm",
            "o'neil",
            "$HOME",
            "a`b",
            "back\\slash",
            "tab\ttab",
            "multi\nline",
            "grüße",
            long.as_str(),
        ] {
            assert!(validate_label(label).is_err(), "{label:?}");
        }
        let max = "x".repeat(64);
        for label in ["launcher", "editor.v2", "super_bar", "a-b", max.as_str()] {
            assert!(validate_label(label).is_ok(), "{label:?}");
        }
    }

    #[test]
    fn keys_reject_empty_long_and_command_metacharacters() {
        let long = "S".repeat(129);
        for keys in [
            "",
            "SUPER;Return",
            "SUPER'x",
            "SU\"PER",
            "SU$PER",
            "SU\\PER",
            "SU`PER",
            "multi\nline",
            long.as_str(),
        ] {
            assert!(validate_keys(keys).is_err(), "{keys:?}");
        }
        for keys in ["SUPER,Return", "Super+Return", "SUPER SHIFT,Return"] {
            assert!(validate_keys(keys).is_ok(), "{keys:?}");
        }
    }

    #[test]
    fn exec_command_appends_the_label_with_a_literal_printf_newline() {
        let queue = Path::new("/run/user/1000/riced/shortcuts-main.queue");
        assert_eq!(
            exec_command("launcher", queue),
            r#"sh -c 'printf %s\n launcher >> "/run/user/1000/riced/shortcuts-main.queue"'"#
        );
    }

    #[test]
    fn i3_frames_encode_magic_length_and_type() {
        assert_eq!(
            i3_frame(RUN_COMMAND, "nop"),
            b"i3-ipc\x03\x00\x00\x00\x00\x00\x00\x00nop".to_vec()
        );
    }

    #[test]
    fn i3_replies_require_success_from_every_entry() {
        assert!(i3_parse(br#"[{"success":true}]"#).is_ok());
        assert_eq!(
            i3_parse(br#"[{"success":true},{"success":false,"error":"no such binding"}]"#),
            Err("no such binding".to_owned())
        );
        assert_eq!(
            i3_parse(br#"[{"success":false}]"#),
            Err("unspecified failure".to_owned())
        );
        assert!(i3_parse(b"garbage").is_err());
        assert!(i3_parse(b"[]").is_err());
    }

    #[test]
    fn queue_drain_forwards_labels_then_truncates_the_file() {
        let dir = std::env::temp_dir().join(format!("riced-shortcuts-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let queue = dir.join("shortcuts-test.queue");
        fs::write(&queue, format!("launcher\n\nmenu\n{}\n", "x".repeat(300))).unwrap();

        let labels = drain_queue(&queue).unwrap();
        assert_eq!(labels, ["launcher", "menu"]);
        assert_eq!(
            fs::metadata(&queue).unwrap().len(),
            0,
            "queue must be truncated"
        );

        // An over-long queue is dropped whole instead of half-parsed.
        fs::write(&queue, vec![b'x'; (MAX_QUEUE + 1) as usize]).unwrap();
        assert_eq!(drain_queue(&queue).unwrap(), Vec::<String>::new());
        assert_eq!(fs::metadata(&queue).unwrap().len(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn snapshots_list_binds_and_can_report_no_backend() {
        let binds = BTreeMap::from([("SUPER,Return".to_owned(), "launcher".to_owned())]);
        let online = snapshot(true, Some("hyprland"), &binds);
        assert_eq!(online["connected"], true);
        assert_eq!(online["compositor"], "hyprland");
        assert_eq!(online["binds"][0]["keys"], "SUPER,Return");
        assert_eq!(online["binds"][0]["label"], "launcher");

        let offline = snapshot(false, None, &BTreeMap::new());
        assert_eq!(offline["connected"], false);
        assert!(offline["compositor"].is_null());
        assert_eq!(offline["binds"], json!([]));
    }
}

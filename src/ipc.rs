use std::{
    env, fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, PartialEq, Eq)]
pub struct Command {
    pub command: String,
    pub args: Vec<String>,
}

fn queue_dir() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("riced")
        .join("ipc")
}

pub fn queue(command: &str, args: &[String]) -> io::Result<()> {
    let dir = queue_dir();
    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    queue_at(&dir, command, args)
}

pub fn take() -> Option<Command> {
    take_from(&queue_dir())
}

fn queue_at(dir: &Path, command: &str, args: &[String]) -> io::Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!(
        "{nanos:020}-{}-{}.ready",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let temp = dir.join(format!("{name}.tmp"));
    let line = serde_json::json!({ "command": command, "args": args });
    let result = fs::write(&temp, serde_json::to_vec(&line)?)
        .and_then(|()| fs::rename(&temp, dir.join(name)));
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

fn take_from(dir: &Path) -> Option<Command> {
    let mut paths = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "ready"))
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let content = fs::read_to_string(&path).ok();
        let _ = fs::remove_file(&path);
        if let Some(command) = content.as_deref().and_then(parse) {
            return Some(command);
        }
    }
    None
}

fn parse(content: &str) -> Option<Command> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    let command = value.get("command")?.as_str().filter(|s| !s.is_empty())?;
    let args = value
        .get("args")?
        .as_array()?
        .iter()
        .map(|arg| arg.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    Some(Command {
        command: command.to_owned(),
        args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn queues_commands_in_order_and_preserves_arguments() {
        let dir = env::temp_dir().join(format!(
            "riced-ipc-{}-{}",
            process::id(),
            TEST.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        queue_at(
            &dir,
            "first",
            &["--anchor".into(), "top bar".into(), "".into()],
        )
        .unwrap();
        queue_at(&dir, "second", &[]).unwrap();

        assert_eq!(
            take_from(&dir),
            Some(Command {
                command: "first".into(),
                args: vec!["--anchor".into(), "top bar".into(), "".into()],
            })
        );
        assert_eq!(
            take_from(&dir),
            Some(Command {
                command: "second".into(),
                args: vec![],
            })
        );
        assert_eq!(take_from(&dir), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn skips_malformed_requests() {
        let dir = env::temp_dir().join(format!(
            "riced-ipc-{}-{}",
            process::id(),
            TEST.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("000.ready"), "not json").unwrap();
        queue_at(&dir, "valid", &[]).unwrap();

        assert_eq!(
            take_from(&dir),
            Some(Command {
                command: "valid".into(),
                args: vec![],
            })
        );
        let _ = fs::remove_dir_all(dir);
    }
}

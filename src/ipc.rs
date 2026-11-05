use std::{
    env, fs, io,
    os::unix::fs::{MetadataExt, PermissionsExt},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Directory identity keeps aliases together and equal basenames distinct.
pub fn profile_id(directory: &Path) -> io::Result<String> {
    let metadata = fs::metadata(directory)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "config path must be a directory",
        ));
    }
    Ok(format!("config-{:x}-{:x}", metadata.dev(), metadata.ino()))
}

#[derive(Debug, PartialEq, Eq)]
pub struct Command {
    pub command: String,
    pub args: Vec<String>,
}

fn runtime_dir() -> io::Result<PathBuf> {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map(|path| path.join("riced"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))
}

fn destination(root: &Path, profile: Option<&str>) -> io::Result<PathBuf> {
    let profile = match profile {
        Some(profile) => {
            crate::config_name(profile)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            UnixStream::connect(root.join(format!("{profile}.sock")))?;
            profile.to_owned()
        }
        None => {
            let mut profiles = Vec::new();
            for entry in fs::read_dir(root)? {
                let path = entry?.path();
                if path.extension().is_some_and(|ext| ext == "sock")
                    && UnixStream::connect(&path).is_ok()
                    && let Some(name) = path.file_stem().and_then(|name| name.to_str())
                {
                    profiles.push(name.to_owned());
                }
            }
            if profiles.len() != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "expected one running profile; use riced ipc -c <name> or -p <directory> <command> [args...]",
                ));
            }
            profiles.remove(0)
        }
    };
    Ok(root.join("ipc").join(profile))
}

pub fn queue(profile: Option<&str>, command: &str, args: &[String]) -> io::Result<()> {
    let dir = destination(&runtime_dir()?, profile)?;
    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    queue_at(&dir, command, args)
}

pub fn take(profile: &str) -> Option<Command> {
    take_from(&runtime_dir().ok()?.join("ipc").join(profile))
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

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

/// Stores the canonical `shell.lua` path for `profile`, so commands running
/// beside a daemon can install into the profile it is supervising.
pub fn record_running_shell(profile: &str, shell: &Path) -> io::Result<()> {
    record_running_shell_at(&runtime_dir()?, profile, shell)
}

fn record_running_shell_at(root: &Path, profile: &str, shell: &Path) -> io::Result<()> {
    fs::create_dir_all(root)?;
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    let path = root.join(format!("{profile}.profile"));
    let temporary = root.join(format!(".{profile}.profile.tmp-{}", process::id()));
    fs::write(&temporary, shell.as_os_str().as_encoded_bytes())?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(&temporary, &path)
}

/// Returns the only live daemon's `shell.lua`, rejecting stale runtime records.
pub fn running_shell() -> io::Result<PathBuf> {
    running_shell_at(&runtime_dir()?)
}

fn running_shell_at(root: &Path) -> io::Result<PathBuf> {
    let profile = running_profile_at(root)?;
    let path = root.join(format!("{profile}.profile"));
    let shell = fs::read_to_string(&path).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "running profile location is missing",
        )
    })?;
    let directory = Path::new(&shell).parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "recorded shell path has no parent",
        )
    })?;
    if profile_id(directory)? != profile
        || Path::new(&shell).file_name().and_then(|name| name.to_str()) != Some("shell.lua")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "running profile location is stale; restart the daemon",
        ));
    }
    Ok(PathBuf::from(shell))
}

pub(crate) fn runtime_dir() -> io::Result<PathBuf> {
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
        None => running_profile_at(root)?,
    };
    Ok(root.join("ipc").join(profile))
}

/// Returns the only running configuration, for commands without `-c` or `-p`.
pub fn running_profile() -> io::Result<String> {
    running_profile_at(&runtime_dir()?)
}

const NO_RUNNING_PROFILE: &str =
    "expected one running profile; use -c <name> or -p <directory> to select a configuration";

fn running_profile_at(root: &Path) -> io::Result<String> {
    let mut profiles = Vec::new();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                NO_RUNNING_PROFILE,
            ));
        }
        Err(error) => return Err(error),
    };
    for entry in entries {
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
            NO_RUNNING_PROFILE,
        ));
    }
    Ok(profiles.remove(0))
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
    fn running_shell_resolves_the_profile_started_by_the_daemon() {
        use std::os::unix::net::UnixListener;

        let root = env::temp_dir().join(format!(
            "riced-running-{}-{}",
            process::id(),
            TEST.fetch_add(1, Ordering::Relaxed)
        ));
        let profile_dir = root.join("config");
        fs::create_dir_all(&profile_dir).unwrap();
        let profile = profile_id(&profile_dir).unwrap();
        let shell = profile_dir.join("shell.lua");
        record_running_shell_at(&root, &profile, &shell).unwrap();
        let _socket = UnixListener::bind(root.join(format!("{profile}.sock"))).unwrap();

        assert_eq!(running_shell_at(&root).unwrap(), shell);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn running_shell_rejects_records_that_do_not_match_the_live_socket() {
        use std::os::unix::net::UnixListener;

        let root = env::temp_dir().join(format!(
            "riced-running-stale-{}-{}",
            process::id(),
            TEST.fetch_add(1, Ordering::Relaxed)
        ));
        let started = root.join("started");
        let recorded = root.join("recorded");
        fs::create_dir_all(&started).unwrap();
        fs::create_dir_all(&recorded).unwrap();
        let profile = profile_id(&started).unwrap();
        record_running_shell_at(&root, &profile, &recorded.join("shell.lua")).unwrap();
        let _socket = UnixListener::bind(root.join(format!("{profile}.sock"))).unwrap();

        assert!(running_shell_at(&root).is_err());
        let _ = fs::remove_dir_all(root);
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

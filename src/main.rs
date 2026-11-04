mod file;
mod ipc;
mod layers;
mod lua;
mod modules;
mod plugins;
mod runtime;
mod update;
mod widgets;

use std::{
    cell::RefCell,
    env,
    path::{Component, Path, PathBuf},
    process::ExitCode,
    rc::Rc,
};

fn profile_path(config: &Path, profile: &std::ffi::OsStr) -> Result<PathBuf, String> {
    let mut components = Path::new(profile).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err("profile must be a single directory name".into());
    }
    Ok(config.join("riced").join(profile).join("shell.lua"))
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let flag = args.next();
    if flag.as_deref() == Some(std::ffi::OsStr::new("--help")) {
        println!(
            "Usage: riced -p <profile>\n       riced ipc <command> [args...]\nLoads $XDG_CONFIG_HOME/riced/<profile>/shell.lua (or ~/.config/riced/...).\nSends an IPC command to the running daemon."
        );
        return Ok(());
    }
    if flag.as_deref() == Some(std::ffi::OsStr::new("ipc")) {
        let command = args
            .next()
            .ok_or("usage: riced ipc <command> [args...]")?
            .into_string()
            .map_err(|_| "IPC command must be valid UTF-8")?;
        if command.is_empty() {
            return Err("usage: riced ipc <command> [args...]".into());
        }
        let args = args
            .map(|arg| {
                arg.into_string()
                    .map_err(|_| "IPC arguments must be valid UTF-8")
            })
            .collect::<Result<Vec<_>, _>>()?;
        ipc::queue(&command, &args)?;
        return Ok(());
    }
    if !matches!(
        flag.as_deref().and_then(|s| s.to_str()),
        Some("-p" | "--profile")
    ) {
        return Err("Usage: riced -p <profile> | riced ipc <command> [args...]".into());
    }
    let profile = args.next().ok_or("missing profile after -p")?;
    if args.next().is_some() {
        return Err("unexpected argument; usage: riced -p <profile>".into());
    }
    let config = env::var_os("XDG_CONFIG_HOME")
        .filter(|p| Path::new(p).is_absolute())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|p| !p.is_empty())
                .map(|p| PathBuf::from(p).join(".config"))
        })
        .ok_or("neither XDG_CONFIG_HOME nor HOME is set")?;
    let path = profile_path(&config, &profile)?;
    let source = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let plugins = Rc::new(RefCell::new(plugins::Plugins::start(&path)?));
    runtime::load_with_plugins(&source, &path, plugins)?.run()?;
    Ok(())
}

fn main() -> ExitCode {
    let _ = riced_extension_sdk::logging::init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            riced_extension_sdk::log::error!(target: "startup.error", "{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn resolves_profile_shell() {
        assert_eq!(
            profile_path(Path::new("/config"), &OsString::from("test")).unwrap(),
            Path::new("/config/riced/test/shell.lua")
        );
    }

    #[test]
    fn rejects_paths_outside_profile() {
        for profile in ["", ".", "..", "../test", "/test", "a/b"] {
            assert!(
                profile_path(Path::new("/config"), &OsString::from(profile)).is_err(),
                "{profile}"
            );
        }
    }
}

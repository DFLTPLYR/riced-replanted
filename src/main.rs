mod file;
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
            "Usage: riced -p <profile>\nLoads $XDG_CONFIG_HOME/riced/<profile>/shell.lua (or ~/.config/riced/...)."
        );
        return Ok(());
    }
    if !matches!(
        flag.as_deref().and_then(|s| s.to_str()),
        Some("-p" | "--profile")
    ) {
        return Err("Usage: riced -p <profile>".into());
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
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("riced: {error}");
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

//! Re-execs the daemon for `riced ipc restart`.

use std::{
    env,
    ffi::{OsStr, OsString},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
};

/// The binary and arguments a restart re-execs.
pub struct Restart {
    exe: PathBuf,
    args: Vec<OsString>,
}

impl Restart {
    /// Resolves the binary this process re-execs.
    ///
    /// `current_exe` is not used: after a binary is replaced by rename,
    /// `/proc/self/exe` points at the unlinked old file.
    pub fn resolve() -> Option<Self> {
        let exe = resolve_exe(
            env::var_os("RICED_EXE").as_deref(),
            env::args_os().next().as_deref(),
            env::var_os("PATH").as_deref(),
        )?;
        Some(Self {
            exe,
            args: env::args_os().skip(1).collect(),
        })
    }

    pub fn exe(&self) -> &Path {
        &self.exe
    }

    /// Replaces this process image. Returns only when the exec failed.
    pub fn exec(&self) -> std::io::Error {
        Command::new(&self.exe).args(&self.args).exec()
    }
}

fn resolve_exe(exe: Option<&OsStr>, arg0: Option<&OsStr>, path: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(exe) = exe.filter(|exe| !exe.is_empty()) {
        return executable(Path::new(exe));
    }
    let arg0 = arg0.filter(|arg0| !arg0.is_empty())?;
    if arg0.to_string_lossy().contains('/') {
        return executable(Path::new(arg0));
    }
    path.into_iter()
        .flat_map(env::split_paths)
        .filter(|directory| !directory.as_os_str().is_empty())
        .find_map(|directory| executable(&directory.join(arg0)))
}

fn executable(path: &Path) -> Option<PathBuf> {
    let allowed = std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false);
    allowed.then(|| std::path::absolute(path).unwrap_or_else(|_| path.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Executable(PathBuf);

    impl Executable {
        fn new(name: &str) -> Self {
            let directory =
                env::temp_dir().join(format!("riced-restart-{}-{}", std::process::id(), name));
            let _ = std::fs::remove_dir_all(&directory);
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("riced");
            std::fs::write(&path, "").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self(path)
        }
    }

    impl Drop for Executable {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
        }
    }

    #[test]
    fn resolves_an_absolute_program_and_prefers_the_environment() {
        let program = Executable::new("absolute");
        let path = OsString::from(program.0.to_str().unwrap());
        assert_eq!(
            resolve_exe(None, Some(&path), None),
            Some(program.0.clone())
        );
        assert_eq!(
            resolve_exe(Some(&path), Some(OsStr::new("/ignored")), None),
            Some(program.0.clone())
        );
        assert_eq!(resolve_exe(None, None, None), None);
        assert_eq!(resolve_exe(None, Some(OsStr::new("")), None), None);
    }

    #[test]
    fn searched_the_path_for_a_bare_program_name() {
        let program = Executable::new("search");
        let path = OsString::from(format!(
            "/nonexistent:{}",
            program.0.parent().unwrap().display()
        ));
        assert_eq!(
            resolve_exe(None, Some(OsStr::new("riced")), Some(&path)),
            Some(program.0.clone())
        );
        assert_eq!(
            resolve_exe(
                None,
                Some(OsStr::new("riced")),
                Some(OsStr::new("/nonexistent"))
            ),
            None
        );
    }

    #[test]
    fn rejects_missing_and_non_executable_programs() {
        let program = Executable::new("mode");
        std::fs::set_permissions(&program.0, std::fs::Permissions::from_mode(0o644)).unwrap();
        let path = OsString::from(program.0.to_str().unwrap());
        assert_eq!(resolve_exe(None, Some(&path), None), None);
        assert_eq!(
            resolve_exe(Some(OsStr::new("/nonexistent/riced")), None, None),
            None
        );
    }
}

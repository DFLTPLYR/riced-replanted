mod file;
mod ipc;
mod layers;
mod logs;
mod lua;
mod modules;
mod plugins;
mod restart;
mod runtime;
mod update;
mod widgets;

use std::{
    cell::RefCell,
    env,
    io::{BufRead, BufReader},
    path::{Component, Path, PathBuf},
    process::{Command as ProcessCommand, ExitCode, Stdio},
    rc::Rc,
    time::Duration,
};

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "riced",
    version,
    about = "Lua-driven Wayland desktop shell",
    after_help = "Without -c or -p, loads $XDG_CONFIG_HOME/riced/shell.lua (or ~/.config/riced/shell.lua).",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(flatten)]
    config: ConfigArgs,
    /// Mirror logs to stderr; records are always stored for `riced logs`
    #[arg(long)]
    logs: bool,
    /// Seconds to keep stored logs; 0 keeps them until cleared
    #[arg(long, value_name = "SECONDS", default_value_t = 30)]
    log_retention: u64,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Args)]
#[group(multiple = false)]
struct ConfigArgs {
    /// Load a named configuration under $XDG_CONFIG_HOME/riced (or ~/.config/riced)
    #[arg(short, long, value_name = "NAME", value_parser = config_name)]
    config: Option<String>,
    /// Load shell.lua from this directory (relative to the current directory)
    #[arg(short, long, value_name = "DIRECTORY")]
    path: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Queue a command for a running daemon; without a selector, require exactly one
    Ipc {
        #[command(flatten)]
        config: ConfigArgs,
        /// Command name followed by arguments passed unchanged to its Lua handler
        #[arg(required = true, num_args = 1.., trailing_var_arg = true, value_name = "COMMAND")]
        command: Vec<String>,
    },
    /// Print stored logs, or clear them with --clear
    Logs {
        #[command(flatten)]
        config: ConfigArgs,
        /// Remove stored logs instead of printing them
        #[arg(long)]
        clear: bool,
    },
    /// Install the core lua-language-server definitions into a profile
    Stubs {
        #[command(flatten)]
        config: ConfigArgs,
    },
    /// Build and package a Rust extension
    Extension {
        /// Required subcommand
        #[command(subcommand)]
        command: ExtensionCommand,
    },
}

#[derive(Subcommand)]
enum ExtensionCommand {
    /// Build a Rust extension and copy it into an output directory
    Build {
        /// Extension package directory, relative to the current directory
        #[arg(value_name = "EXTENSION", id = "extension")]
        path: PathBuf,
        /// Required subcommand
        #[command(flatten)]
        config: ConfigArgs,
        /// Copy the executable and Lua companions into this directory. Without
        /// -o and without -c or -p, use the running profile's extensions directory.
        #[arg(short, long, value_name = "OUTPUT")]
        output: Option<PathBuf>,
    },
}

fn config_name(value: &str) -> Result<String, String> {
    let mut components = Path::new(value).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err("config must be a single directory name".into());
    }
    Ok(value.to_owned())
}

impl ConfigArgs {
    fn extensions_dir(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let directory = if self.config.is_some() || self.path.is_some() {
            self.directory()?
        } else {
            let shell = ipc::running_shell().map_err(|_| {
                "no running profile with a recorded shell path; use -o <output-path>, -c <name>, -p <directory>, or start riced first"
            })?;
            shell.parent().unwrap().to_path_buf()
        };
        Ok(directory.join("extensions"))
    }

    fn directory(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
        if let Some(path) = &self.path {
            if path.as_os_str().is_empty() {
                return Err("config path must not be empty".into());
            }
            return Ok(std::path::absolute(path)?);
        }
        let root = env::var_os("XDG_CONFIG_HOME")
            .filter(|p| Path::new(p).is_absolute())
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os("HOME")
                    .filter(|p| !p.is_empty())
                    .map(|p| PathBuf::from(p).join(".config"))
            })
            .ok_or("neither XDG_CONFIG_HOME nor HOME is set")?;
        Ok(std::path::absolute(
            root.join("riced")
                .join(self.config.as_deref().unwrap_or("")),
        )?)
    }
}

fn build_extension(
    config: &ConfigArgs,
    path: &Path,
    output: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    if path.as_os_str().is_empty() || output.is_some_and(|output| output.as_os_str().is_empty()) {
        return Err("extension and output paths must not be empty".into());
    }
    let path = std::path::absolute(path)?;
    let output = match output {
        Some(output) => std::path::absolute(output)?,
        None => config.extensions_dir()?,
    };
    let manifest = path.join("Cargo.toml");
    let metadata: PackageMetadata = serde_json::from_value(cargo_metadata(&manifest)?)?;
    let package = metadata
        .packages
        .iter()
        .find(|package| package.manifest_path == manifest)
        .ok_or_else(|| format!("{} is not a Cargo package directory", path.display()))?;
    let targets: Vec<&TargetMetadata> = package
        .targets
        .iter()
        .filter(|target| target.kind == ["bin"])
        .collect();
    if targets.is_empty() {
        return Err(format!("{} does not contain a Cargo binary", path.display()).into());
    }
    if targets.len() > 1 {
        return Err(format!(
            "{} contains multiple Cargo binaries; split each extension into its own package",
            path.display()
        )
        .into());
    }
    let target = targets[0];

    let mut child = ProcessCommand::new("cargo")
        .args(["build", "--release", "--message-format=json"])
        .arg("--bin")
        .arg(&target.name)
        .arg("--manifest-path")
        .arg(&manifest)
        .env("CARGO_TERM_COLOR", "never")
        .stdout(Stdio::piped())
        .spawn()?;
    let child_stdout = child.stdout.take().ok_or("cargo build stdout is missing")?;
    let mut executable = None;
    for line in BufReader::new(child_stdout).lines() {
        let line = line?;
        if !line.starts_with('{') {
            eprintln!("{line}");
            continue;
        }
        let message: serde_json::Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(_) => continue,
        };
        if message["reason"] == "compiler-artifact"
            && message["target"]["name"] == target.name
            && message["filenames"]
                .as_array()
                .and_then(|filenames| {
                    filenames.iter().find(|filename| {
                        filename.as_str().is_some_and(|filename| {
                            Path::new(filename)
                                .file_name()
                                .and_then(|name| name.to_str())
                                == Some(target.name.as_str())
                        })
                    })
                })
                .is_some()
        {
            executable = message["filenames"].as_array().and_then(|filenames| {
                filenames
                    .iter()
                    .filter_map(|filename| filename.as_str().map(str::to_owned))
                    .find(|filename| {
                        Path::new(filename)
                            .file_name()
                            .and_then(|name| name.to_str())
                            == Some(target.name.as_str())
                    })
            });
        }
    }
    if !child.wait()?.success() {
        return Err("cargo build failed".into());
    }
    let executable = executable.ok_or("cargo build did not produce the extension binary")?;

    std::fs::create_dir_all(&output)?;
    let destination = output.join(&target.name);
    // A running extension holds its executable open, so an in-place copy fails
    // with ETXTBSY. Install through a temporary file and rename over it instead.
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = output.join(format!(
        ".{}.tmp-{}-{suffix}",
        target.name,
        std::process::id()
    ));
    let installed = (|| -> std::io::Result<()> {
        std::fs::copy(executable, &temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&temporary, &destination)
    })();
    if let Err(error) = installed {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    install_lua(&path, &output)?;
    Ok(())
}

/// Copies the package's Lua companions into `output`, skipping an in-place install.
fn install_lua(package: &Path, output: &Path) -> std::io::Result<()> {
    if std::fs::canonicalize(package)? == std::fs::canonicalize(output)? {
        return Ok(());
    }
    for entry in std::fs::read_dir(package)? {
        let entry = entry?;
        let source = entry.path();
        if !source.is_file()
            || source
                .extension()
                .is_none_or(|extension| extension != "lua")
        {
            continue;
        }
        let destination = output.join(source.file_name().unwrap());
        std::fs::copy(&source, &destination)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o644))?;
        }
    }
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
struct PackageMetadata {
    packages: Vec<MetadataPackage>,
}

#[derive(Debug, serde::Deserialize)]
struct MetadataPackage {
    manifest_path: PathBuf,
    targets: Vec<TargetMetadata>,
}

#[derive(Debug, serde::Deserialize)]
struct TargetMetadata {
    name: String,
    kind: Vec<String>,
}

fn cargo_metadata(manifest: &Path) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let output = ProcessCommand::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .arg("--manifest-path")
        .arg(manifest)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "cannot read extension package {}: {}",
            manifest.display(),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

const CORE_STUB: &str = include_str!("../example/stubs/riced.lua");
const CORE_LUARC: &str = include_str!("../example/.luarc.json");

fn install_stubs(config: &ConfigArgs) -> Result<(), Box<dyn std::error::Error>> {
    let directory = config.directory()?;
    let stubs = directory.join("stubs");
    std::fs::create_dir_all(&stubs)?;
    std::fs::write(stubs.join("riced.lua"), CORE_STUB)?;
    // Keep an existing editor config: it may contain user settings.
    let luarc = directory.join(".luarc.json");
    if !luarc.exists() {
        std::fs::write(luarc, CORE_LUARC)?;
    }
    Ok(())
}

fn selected_profile(config: &ConfigArgs) -> Result<Option<String>, Box<dyn std::error::Error>> {
    if config.path.is_some() || config.config.is_some() {
        Ok(Some(ipc::profile_id(&config.directory()?)?))
    } else {
        Ok(None)
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(command) = cli.command {
        match command {
            Command::Ipc { config, command } => {
                let (command, args) = command.split_first().ok_or("missing IPC command")?;
                if command.is_empty() {
                    return Err("IPC command must not be empty".into());
                }
                ipc::queue(selected_profile(&config)?.as_deref(), command, args)?;
            }
            Command::Logs { config, clear } => {
                let profile = match selected_profile(&config)? {
                    Some(profile) => profile,
                    None => ipc::running_profile()?,
                };
                logs::show(&profile, clear)?;
            }
            Command::Stubs { config } => install_stubs(&config)?,
            Command::Extension { command } => match command {
                ExtensionCommand::Build {
                    config,
                    path,
                    output,
                } => build_extension(&config, &path, output.as_deref())?,
            },
        }
        return Ok(());
    }
    let directory = cli.config.directory()?;
    let path = directory.join("shell.lua");
    let source = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    // Extensions and module reloads must keep resolving from this directory.
    let path = directory.canonicalize()?.join("shell.lua");
    let plugins = Rc::new(RefCell::new(plugins::Plugins::start(&path)?));
    // After Plugins::start clears stale runtime state for this profile.
    ipc::record_running_shell(&ipc::profile_id(&directory)?, &path)?;
    logs::start(
        &ipc::profile_id(&directory)?,
        (cli.log_retention > 0).then(|| Duration::from_secs(cli.log_retention)),
        cli.logs,
    )?;
    runtime::load_with_plugins(&source, &path, plugins)?
        .with_restart(restart::Restart::resolve())
        .run()?;
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Fatal startup errors stay visible even when routine logs are quiet.
            eprintln!("[startup.error]: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_outside_profile() {
        for profile in ["", ".", "..", "../test", "/test", "a/b"] {
            assert!(config_name(profile).is_err(), "{profile}");
        }
        assert_eq!(config_name("test").unwrap(), "test");
    }

    #[test]
    fn cli_definition_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn in_place_extension_build_keeps_lua_sources() {
        let dir = std::env::temp_dir().join(format!("riced-install-lua-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("shell.lua");
        std::fs::write(&script, "return 1\n").unwrap();
        install_lua(&dir, &dir).unwrap();
        assert_eq!(std::fs::read_to_string(&script).unwrap(), "return 1\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

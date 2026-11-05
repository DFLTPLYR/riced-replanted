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
}

fn config_name(value: &str) -> Result<String, String> {
    let mut components = Path::new(value).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err("config must be a single directory name".into());
    }
    Ok(value.to_owned())
}

impl ConfigArgs {
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

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(Command::Ipc { config, command }) = cli.command {
        let (command, args) = command.split_first().ok_or("missing IPC command")?;
        if command.is_empty() {
            return Err("IPC command must not be empty".into());
        }
        let profile = if config.path.is_some() || config.config.is_some() {
            Some(ipc::profile_id(&config.directory()?)?)
        } else {
            None
        };
        ipc::queue(profile.as_deref(), command, args)?;
        return Ok(());
    }
    let directory = cli.config.directory()?;
    let path = directory.join("shell.lua");
    let source = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    // Extensions and module reloads must keep resolving from this directory.
    let path = directory.canonicalize()?.join("shell.lua");
    let plugins = Rc::new(RefCell::new(plugins::Plugins::start(&path)?));
    runtime::load_with_plugins(&source, &path, plugins)?.run()?;
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let _ = riced_extension_sdk::logging::init();
    match run(cli) {
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
}

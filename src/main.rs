mod file;
mod ipc;
mod layers;
mod logs;
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
    logs::start(
        &ipc::profile_id(&directory)?,
        (cli.log_retention > 0).then(|| Duration::from_secs(cli.log_retention)),
        cli.logs,
    )?;
    runtime::load_with_plugins(&source, &path, plugins)?.run()?;
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
}

//! Shared stderr logging in `[event]: information` format.
//!
//! Use the re-exported `log` macros with `target:` as the event name.

use std::io::{self, IsTerminal, Write};

use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

struct Logger;

static LOGGER: Logger = Logger;

/// Installs the stderr logger with info, warning, and error output enabled.
/// Returns an error without changing an already installed logger or its filter.
/// Call `log::set_max_level` afterwards to change verbosity.
pub fn init() -> Result<(), SetLoggerError> {
    log::set_logger(&LOGGER)?;
    log::set_max_level(LevelFilter::Info);
    Ok(())
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            let mut stderr = io::stderr().lock();
            let color = stderr.is_terminal() && std::env::var_os("NO_COLOR").is_none();
            // A closed stderr must not turn a diagnostic into an application failure.
            let _ = write_record(&mut stderr, record, color);
        }
    }

    fn flush(&self) {
        let _ = io::stderr().lock().flush();
    }
}

fn write_record(writer: &mut impl Write, record: &Record<'_>, color: bool) -> io::Result<()> {
    if color {
        let code = match record.level() {
            Level::Error => 31,
            Level::Warn => 33,
            Level::Info => 32,
            Level::Debug => 36,
            Level::Trace => 90,
        };
        write!(writer, "\x1b[{code}m[{}]\x1b[0m", record.target())?;
    } else {
        write!(writer, "[{}]", record.target())?;
    }
    writeln!(writer, ": {}", record.args())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_event_and_information() {
        let mut output = Vec::new();
        write_record(
            &mut output,
            &Record::builder()
                .target("extension.started")
                .level(log::Level::Info)
                .args(format_args!("started {}", "dbus"))
                .build(),
            false,
        )
        .unwrap();
        assert_eq!(output, b"[extension.started]: started dbus\n");
    }

    #[test]
    fn colors_only_the_event_label_by_severity() {
        for (level, code) in [
            (Level::Error, 31),
            (Level::Warn, 33),
            (Level::Info, 32),
            (Level::Debug, 36),
            (Level::Trace, 90),
        ] {
            let mut output = Vec::new();
            write_record(
                &mut output,
                &Record::builder()
                    .target("event")
                    .level(level)
                    .args(format_args!("information"))
                    .build(),
                true,
            )
            .unwrap();
            assert_eq!(
                output,
                format!("\x1b[{code}m[event]\x1b[0m: information\n").as_bytes()
            );
        }
    }

    #[test]
    fn initialization_preserves_an_existing_logger_and_filter() {
        init().unwrap();
        log::set_max_level(LevelFilter::Warn);
        assert!(init().is_err());
        assert_eq!(log::max_level(), LevelFilter::Warn);
        assert!(!log::log_enabled!(log::Level::Info));
        assert!(log::log_enabled!(log::Level::Error));
    }
}

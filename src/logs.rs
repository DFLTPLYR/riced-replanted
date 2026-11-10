//! Buffered diagnostics shared by the daemon and `riced logs`.
//!
//! Every record goes to `$XDG_RUNTIME_DIR/riced/logs/<profile>.log`. `--logs`
//! also mirrors it to stderr; without that flag the daemon is quiet.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, IsTerminal, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    sync::Mutex,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use riced_extension_sdk::log;
use riced_extension_sdk::log::{LevelFilter, Log, Metadata, Record};

struct Sink {
    path: PathBuf,
    file: File,
    retention: Option<Duration>,
    last_prune: Instant,
    mirror: bool,
}

static SINK: Mutex<Option<Sink>> = Mutex::new(None);
static LOGGER: Logger = Logger;

struct Logger;

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut sink = SINK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(sink) = sink.as_mut() else {
            return;
        };
        if sink.mirror {
            let mut stderr = io::stderr().lock();
            let color = stderr.is_terminal() && std::env::var_os("NO_COLOR").is_none();
            // A closed stderr must not turn a diagnostic into an application failure.
            let _ = riced_extension_sdk::logging::write_record(&mut stderr, record, color);
        }
        write_line(sink, &format!("[{}]: {}", record.target(), record.args()));
    }

    fn flush(&self) {
        let _ = io::stderr().lock().flush();
        if let Some(sink) = SINK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_mut()
        {
            let _ = sink.file.flush();
        }
    }
}

/// Starts the daemon's log sink, moving the previous run's file aside.
pub fn start(profile: &str, retention: Option<Duration>, mirror: bool) -> io::Result<()> {
    let root = crate::ipc::runtime_dir()?;
    fs::create_dir_all(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    let directory = root.join("logs");
    fs::create_dir_all(&directory)?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    let path = directory.join(format!("{profile}.log"));
    let previous = directory.join(format!("{profile}.previous.log"));
    // Keep the last run's records for post-mortem instead of truncating them.
    // The next start replaces this file, so at most one run is retained.
    let _ = fs::rename(&path, &previous);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)?;
    *SINK.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Sink {
        path,
        file,
        retention,
        last_prune: Instant::now(),
        mirror,
    });
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(LevelFilter::Info);
    if retention.is_some() {
        thread::Builder::new()
            .name("riced-logs".into())
            .stack_size(128 * 1024)
            .spawn(|| {
                loop {
                    thread::sleep(Duration::from_secs(1));
                    prune();
                }
            })?;
    }
    Ok(())
}

/// Forwards one line produced by an extension's own logger.
pub fn external(target: &str, message: &str) {
    let line = format!("[{target}]: {message}");
    let mut sink = SINK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(sink) = sink.as_mut() else {
        return;
    };
    if sink.mirror {
        eprintln!("{line}");
    }
    write_line(sink, &line);
}

/// Prints stored logs, or truncates the file when `clear` is set.
pub fn show(profile: &str, clear: bool) -> io::Result<()> {
    let path = crate::ipc::runtime_dir()?
        .join("logs")
        .join(format!("{profile}.log"));
    let mut file = match OpenOptions::new().read(true).write(clear).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if clear {
        file.lock()?;
        return file.set_len(0);
    }
    file.lock_shared()?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    drop(file);
    let mut stdout = io::stdout().lock();
    for line in contents.lines() {
        // Stored lines start with an epoch-millisecond stamp; the CLI shows the
        // same `[event]: information` shape as `--logs`.
        writeln!(
            stdout,
            "{}",
            line.split_once(' ').map_or(line, |(_, line)| line)
        )?;
    }
    Ok(())
}

/// Drops expired lines from the background prune thread.
fn prune() {
    let mut sink = SINK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(sink) = sink.as_mut() else {
        return;
    };
    let Some(retention) = sink.retention else {
        return;
    };
    if sink.last_prune.elapsed() < Duration::from_secs(1) {
        return;
    }
    sink.last_prune = Instant::now();
    let _ = prune_sink(sink, retention);
}

fn write_line(sink: &mut Sink, line: &str) {
    if sink.file.lock().is_err() {
        return;
    }
    let stamp = now_millis();
    for line in line.split('\n') {
        let _ = writeln!(sink.file, "{stamp} {line}");
    }
    let _ = sink.file.flush();
    let _ = sink.file.unlock();
}

fn prune_sink(sink: &mut Sink, retention: Duration) -> io::Result<()> {
    // The CLI runs in another process, so hold a file lock across read and rewrite.
    let mut file = OpenOptions::new().read(true).write(true).open(&sink.path)?;
    file.lock()?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let kept = prune_lines(
        &contents,
        now_millis().saturating_sub(retention.as_millis()),
    );
    if kept.len() == contents.len() {
        return Ok(());
    }
    let temp = sink.path.with_extension("log.tmp");
    let result = (|| -> io::Result<File> {
        let mut out = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        out.write_all(kept.as_bytes())?;
        out.sync_all()?;
        // Keep the append handle on the same inode so no write lands in the
        // replaced file between rename and reopen.
        let append = OpenOptions::new().append(true).open(&temp)?;
        fs::rename(&temp, &sink.path)?;
        Ok(append)
    })();
    match result {
        Ok(file) => {
            sink.file = file;
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

fn prune_lines(contents: &str, cutoff: u128) -> String {
    let mut kept = String::with_capacity(contents.len());
    for line in contents.lines() {
        let recent = line
            .split_once(' ')
            .and_then(|(stamp, _)| stamp.parse::<u128>().ok())
            .is_some_and(|stamp| stamp >= cutoff);
        if recent {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    kept
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_sink(name: &str) -> Sink {
        let path = std::env::temp_dir().join(format!("riced-logs-{name}-{}", std::process::id()));
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .unwrap();
        Sink {
            path,
            file,
            retention: None,
            last_prune: Instant::now(),
            mirror: false,
        }
    }

    #[test]
    fn multiline_records_survive_pruning_and_expire_together() {
        let mut sink = test_sink("multiline");
        writeln!(sink.file, "0 expired").unwrap();
        write_line(&mut sink, "[lua]: first\nsecond line\n\nlast line\n");
        prune_sink(&mut sink, Duration::from_secs(30)).unwrap();
        let contents = fs::read_to_string(&sink.path).unwrap();
        let lines: Vec<_> = contents.lines().collect();
        let stamp = lines[0].split_once(' ').unwrap().0;
        assert_eq!(
            contents,
            format!(
                "{stamp} [lua]: first\n{stamp} second line\n{stamp} \n{stamp} last line\n{stamp} \n"
            )
        );
        assert!(prune_lines(&contents, stamp.parse::<u128>().unwrap() + 1).is_empty());
        fs::remove_file(&sink.path).unwrap();
    }

    #[test]
    fn pruning_waits_for_a_clear_and_does_not_restore_records() {
        let mut sink = test_sink("prune-lock");
        writeln!(sink.file, "0 expired").unwrap();
        write_line(&mut sink, "[lua]: retained");
        let clear = OpenOptions::new().write(true).open(&sink.path).unwrap();
        clear.lock().unwrap();
        std::thread::scope(|scope| {
            let (started_tx, started_rx) = std::sync::mpsc::channel();
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let sink = &mut sink;
            let prune = scope.spawn(move || {
                started_tx.send(()).unwrap();
                prune_sink(sink, Duration::from_secs(30)).unwrap();
                done_tx.send(()).unwrap();
            });
            started_rx.recv().unwrap();
            let waiting = done_rx.recv_timeout(Duration::from_millis(100)).is_err();
            clear.set_len(0).unwrap();
            clear.unlock().unwrap();
            prune.join().unwrap();
            assert!(waiting, "prune finished before clearing released its lock");
        });
        assert!(fs::read(&sink.path).unwrap().is_empty());
        fs::remove_file(&sink.path).unwrap();
    }

    #[test]
    fn pruning_keeps_recent_lines_and_drops_malformed_ones() {
        let contents = "9990 old\n9995 recent\nnot a log line\n";
        assert_eq!(prune_lines(contents, 9992), "9995 recent\n");
    }
}

//! XDG desktop entries: publishes installed applications and launches them.

use std::{
    collections::HashSet,
    env, fs, io,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use freedesktop_entry_parser::Entry;
use riced_extension_sdk::{Plugin, bounded_array, protocol::MAX_LINE};
use serde_json::{Value, json};

const REFRESH: Duration = Duration::from_secs(30);
const MAX_SCAN_DEPTH: usize = 4;
const MAX_NAME: usize = 256;
const MAX_COMMENT: usize = 1024;

#[derive(Clone)]
struct Application {
    id: String,
    name: String,
    icon: String,
    comment: String,
    exec: String,
    terminal: bool,
    path: PathBuf,
}

fn main() -> io::Result<()> {
    let applications = Arc::new(Mutex::new(Vec::new()));
    let launch = Arc::clone(&applications);
    Plugin::new("applications")
        .state(["applications"])
        .method("launch", false, move |args| launch_app(&launch, args))
        .serve(move |context| {
            let mut last = Value::Null;
            loop {
                let next = scan();
                let snapshot = {
                    let mut applications = applications.lock().unwrap();
                    *applications = next;
                    snapshot(&applications)
                };
                if snapshot != last {
                    context.publish(snapshot.clone());
                    last = snapshot;
                }
                thread::sleep(REFRESH);
            }
        })
}

fn launch_app(applications: &Arc<Mutex<Vec<Application>>>, args: &Value) -> Result<Value, String> {
    let id = args
        .get(0)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "missing application id".to_owned())?;
    let app = applications
        .lock()
        .map_err(|_| "application list lock poisoned".to_owned())?
        .iter()
        .find(|app| app.id == id)
        .cloned()
        .ok_or_else(|| format!("unknown application: {id}"))?;
    let mut argv = command(&app)?;
    let mut command = Command::new(argv.remove(0));
    command
        .args(argv)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot launch {}: {error}", app.name))?;
    // Reap the child; this process lives as long as the shell, so an unwaited
    // child would stay a zombie after the app exits.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(Value::Null)
}

/// Launches into a new session so riced's death and terminal/process-group
/// signals cannot reach the application.
fn detach(command: &mut Command) {
    // SAFETY: setsid is async-signal-safe and runs between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

fn command(app: &Application) -> Result<Vec<String>, String> {
    let mut argv = exec_arguments(&app.exec, app);
    if argv.is_empty() {
        return Err(format!("{} has no command", app.name));
    }
    if app.terminal {
        let terminal = env::var("TERMINAL")
            .ok()
            .filter(|terminal| !terminal.trim().is_empty())
            .unwrap_or_else(|| "x-terminal-emulator".to_owned());
        let mut parts = split_words(&terminal, true);
        if parts.is_empty() {
            return Err(format!(
                "{} needs a terminal but TERMINAL is empty",
                app.name
            ));
        }
        parts.push("-e".to_owned());
        parts.extend(argv);
        argv = parts;
    }
    Ok(argv)
}

/// Splits a desktop `Exec` value without a shell and expands field codes.
fn exec_arguments(exec: &str, app: &Application) -> Vec<String> {
    split_words(exec, false)
        .into_iter()
        .filter_map(|word| expand_field_codes(&word, app))
        .collect()
}

/// Splits on whitespace, honouring double quotes and backslash escapes.
/// `$TERMINAL` also accepts single quotes so options can contain spaces.
fn split_words(value: &str, single_quotes: bool) -> Vec<String> {
    let mut words = Vec::new();
    let mut token = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' => {
                if let Some(escaped) = chars.next() {
                    token.push(escaped);
                    started = true;
                }
            }
            delimiter @ ('"' | '\'') if single_quotes || delimiter == '"' => {
                match quote {
                    Some(open) if open == delimiter => quote = None,
                    Some(_) => token.push(delimiter),
                    None => quote = Some(delimiter),
                }
                started = true;
            }
            character if character.is_whitespace() && quote.is_none() => {
                if started {
                    words.push(std::mem::take(&mut token));
                    started = false;
                }
            }
            character => {
                token.push(character);
                started = true;
            }
        }
    }
    if started {
        words.push(token);
    }
    words
}

/// Expands one token's field codes. `None` means it only contained codes this
/// launcher omits (file and icon arguments).
fn expand_field_codes(word: &str, app: &Application) -> Option<String> {
    let mut token = String::new();
    let mut produced = word.is_empty();
    let mut chars = word.chars();
    while let Some(character) = chars.next() {
        match character {
            '%' => match chars.next() {
                Some('%') => {
                    token.push('%');
                    produced = true;
                }
                // No icon override or file arguments: omit %i, %f, %F, %u, %U and
                // the deprecated codes %d, %D, %n, %N, %v, %m, %V.
                Some('i' | 'f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm' | 'V') => {}
                Some('c') => {
                    token.push_str(&app.name);
                    produced = true;
                }
                Some('k') => {
                    token.push_str(&app.path.to_string_lossy());
                    produced = true;
                }
                Some(other) => {
                    token.push('%');
                    token.push(other);
                    produced = true;
                }
                None => {
                    token.push('%');
                    produced = true;
                }
            },
            character => {
                token.push(character);
                produced = true;
            }
        }
    }
    produced.then_some(token)
}

fn scan() -> Vec<Application> {
    scan_roots(&data_dirs())
}

fn data_dirs() -> Vec<PathBuf> {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let data_home = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|home| home.join(".local/share")));
    let data_dirs = env::var_os("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let mut seen = HashSet::new();
    data_home
        .into_iter()
        .chain(env::split_paths(&data_dirs).filter(|path| path.is_absolute()))
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

fn scan_roots(roots: &[PathBuf]) -> Vec<Application> {
    let mut seen = HashSet::new();
    let mut applications = Vec::new();
    for root in roots {
        let directory = root.join("applications");
        collect(&directory, &directory, 0, &mut seen, &mut applications);
    }
    applications.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    applications
}

fn collect(
    root: &Path,
    directory: &Path,
    depth: usize,
    seen: &mut HashSet<String>,
    applications: &mut Vec<Application>,
) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect(root, &path, depth + 1, seen, applications);
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(id) = name.strip_suffix(".desktop") else {
            continue;
        };
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let id = relative
            .with_file_name(id)
            .to_string_lossy()
            .replace('/', "-");
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(application) = parse(&path, id) {
            applications.push(application);
        }
    }
}

fn parse(path: &Path, id: String) -> Option<Application> {
    let entry = Entry::parse_file(path).ok()?;
    let section = entry.section("Desktop Entry");
    if section.attr("Type").unwrap_or("Application") != "Application"
        || flag(&section, "Hidden")
        || flag(&section, "NoDisplay")
    {
        return None;
    }
    let name = section.attr("Name")?.trim();
    let exec = section.attr("Exec")?.trim();
    if name.is_empty() || exec.is_empty() {
        return None;
    }
    Some(Application {
        id,
        name: clamp(name, MAX_NAME),
        icon: clamp(section.attr("Icon").unwrap_or("").trim(), MAX_NAME),
        comment: clamp(section.attr("Comment").unwrap_or("").trim(), MAX_COMMENT),
        exec: exec.to_owned(),
        terminal: flag(&section, "Terminal"),
        path: path.to_owned(),
    })
}

fn flag<T: AsRef<str>>(section: &freedesktop_entry_parser::AttrSelector<'_, T>, key: &str) -> bool {
    section.attr(key) == Some("true")
}

fn clamp(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn snapshot(applications: &[Application]) -> Value {
    let items = applications.iter().map(|application| {
        json!({
            "id": application.id,
            "name": application.name,
            "icon": if application.icon.is_empty() { Value::Null } else { json!(application.icon) },
            "comment": if application.comment.is_empty() { Value::Null } else { json!(application.comment) },
            "terminal": application.terminal,
        })
    });
    json!({ "applications": bounded_array(items, MAX_LINE / 2) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn launched_apps_start_in_their_own_session() {
        let mut command = Command::new("sleep");
        command.arg("5");
        detach(&mut command);
        let mut child = command.spawn().unwrap();
        let pid = child.id() as i32;
        assert_eq!(unsafe { libc::getsid(pid) }, pid);
        let _ = child.kill();
        let _ = child.wait();
    }

    fn application(exec: &str) -> Application {
        Application {
            id: "app".into(),
            name: "My App".into(),
            icon: String::new(),
            comment: String::new(),
            exec: exec.into(),
            terminal: false,
            path: PathBuf::from("/usr/share/applications/app.desktop"),
        }
    }

    #[test]
    fn splits_exec_and_expands_field_codes() {
        assert_eq!(
            exec_arguments(
                r#""/opt/My App/run" --title "Hello %c" %U %% %k"#,
                &application("")
            ),
            [
                "/opt/My App/run",
                "--title",
                "Hello My App",
                "%",
                "/usr/share/applications/app.desktop"
            ]
        );
        assert_eq!(
            exec_arguments("app %i %f --flag", &application("")),
            ["app", "--flag"]
        );
        assert!(exec_arguments("%f", &application("")).is_empty());
    }

    #[test]
    fn splits_terminal_options_with_quotes() {
        assert_eq!(
            split_words("kitty --session 'my session'", true),
            ["kitty", "--session", "my session"]
        );
        assert_eq!(
            split_words(r#"kitty --session "my session""#, true),
            ["kitty", "--session", "my session"]
        );
        assert_eq!(
            split_words(r#"kitty -e "my session""#, false),
            ["kitty", "-e", "my session"]
        );
        assert_eq!(
            split_words(r#"kitty --title "Bob's shell""#, true),
            ["kitty", "--title", "Bob's shell"]
        );
        assert_eq!(
            split_words(r#"kitty --title 'say "hi"'"#, true),
            ["kitty", "--title", "say \"hi\""]
        );
    }

    #[test]
    fn exec_keeps_single_quotes_literal() {
        assert_eq!(
            exec_arguments("sh -c 'echo hi'", &application("")),
            ["sh", "-c", "'echo", "hi'"]
        );
    }

    #[test]
    fn scans_roots_in_precedence_order_and_skips_hidden_entries() {
        let root = env::temp_dir().join(format!(
            "riced-applications-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        for (name, body) in [
            (
                "user/applications/dup.desktop",
                "[Desktop Entry]\nName=User\nExec=user\n",
            ),
            (
                "system/applications/dup.desktop",
                "[Desktop Entry]\nName=System\nExec=system\n",
            ),
            (
                "system/applications/hidden.desktop",
                "[Desktop Entry]\nName=Hidden\nExec=hidden\nNoDisplay=true\n",
            ),
            (
                "system/applications/tool.desktop",
                "[Desktop Entry]\nType=Application\nName=Tool\nExec=tool %U\n",
            ),
        ] {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }
        let applications = scan_roots(&[root.join("user"), root.join("system")]);
        let names = applications
            .iter()
            .map(|application| application.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["Tool", "User"]);
        let _ = fs::remove_dir_all(&root);
    }
}

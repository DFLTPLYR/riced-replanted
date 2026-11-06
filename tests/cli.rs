use std::{os::unix::fs::MetadataExt, path::Path, process::Command};

fn profile_id(directory: &Path) -> String {
    let metadata = std::fs::metadata(directory).unwrap();
    format!("config-{:x}-{:x}", metadata.dev(), metadata.ino())
}

#[test]
fn missing_profile_reports_shell_path_and_fails() {
    let config = format!("/tmp/opencode/riced-missing-config-{}", std::process::id());
    let output = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env("XDG_CONFIG_HOME", &config)
        .args(["-c", "test"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("[startup.error]: "));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains(&format!("cannot read {config}/riced/test/shell.lua"))
    );
}

#[test]
fn lua_log_uses_shared_stderr_logging_and_lua_tostring() {
    let directory = std::path::PathBuf::from(format!(
        "/tmp/opencode/riced-lua-log-{}",
        std::process::id()
    ));
    let profile = directory.join("riced/test");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(
        profile.join("shell.lua"),
        r#"
        local settings = function(self, args)
            assert(#args == 0)
            log("called", 42, true, nil, setmetatable({}, {
                __tostring = function() return "custom" end
            }))
            assert(select('#', log()) == 0)
        end
        riced.ipc({settings = settings})
        settings({}, {})
        -- Stop before starting the compositor-dependent runtime.
        error("logging test complete")
    "#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env("XDG_CONFIG_HOME", &directory)
        .env("XDG_RUNTIME_DIR", &directory)
        .args(["--logs", "-c", "test"])
        .output()
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        stderr.starts_with("[lua]: called\t42\ttrue\tnil\tcustom\n[lua]: \n"),
        "{stderr}"
    );
    assert!(stderr.contains("logging test complete"), "{stderr}");
    assert!(!stderr.contains('\x1b'));
}

#[test]
fn lua_log_table_formats_nested_tables() {
    let directory = std::path::PathBuf::from(format!(
        "/tmp/opencode/riced-lua-log-table-{}",
        std::process::id()
    ));
    let profile = directory.join("riced/test");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(
        profile.join("shell.lua"),
        r#"
        log.table({
            name = "test",
            nested = { 1, 2 },
            empty = {},
        })
        -- Stop before starting the compositor-dependent runtime.
        error("logging test complete")
    "#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env("XDG_CONFIG_HOME", &directory)
        .env("XDG_RUNTIME_DIR", &directory)
        .args(["--logs", "-c", "test"])
        .output()
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.starts_with(
            "[lua]: {\n  empty = {},\n  name = \"test\",\n  nested = {\n    1,\n    2,\n  },\n}\n"
        ),
        "{stderr}"
    );
}

#[test]
fn ipc_queues_command_with_arguments() {
    let runtime = format!("/tmp/opencode/riced-ipc-{}", std::process::id());
    let _ = std::fs::remove_dir_all(&runtime);
    std::fs::create_dir_all(format!("{runtime}/riced")).unwrap();
    let _listener =
        std::os::unix::net::UnixListener::bind(format!("{runtime}/riced/test.sock")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env("XDG_RUNTIME_DIR", &runtime)
        .args(["ipc", "settings", "--anchor", "top bar", ""])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let entry = std::fs::read_dir(format!("{runtime}/riced/ipc/test"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(entry.path()).unwrap()).unwrap();
    assert_eq!(value["command"], "settings");
    assert_eq!(
        value["args"],
        serde_json::json!(["--anchor", "top bar", ""])
    );
    let _ = std::fs::remove_dir_all(&runtime);
}

#[test]
fn ipc_targets_one_of_several_running_profiles() {
    let runtime = format!("/tmp/opencode/riced-ipc-profiles-{}", std::process::id());
    let _ = std::fs::remove_dir_all(&runtime);
    std::fs::create_dir_all(format!("{runtime}/riced")).unwrap();
    let one = Path::new(&runtime).join("configs/riced/one");
    let two = Path::new(&runtime).join("configs/riced/two");
    std::fs::create_dir_all(&one).unwrap();
    std::fs::create_dir_all(&two).unwrap();
    let one_id = profile_id(&one);
    let two_id = profile_id(&two);
    let _one =
        std::os::unix::net::UnixListener::bind(format!("{runtime}/riced/{one_id}.sock")).unwrap();
    let _two =
        std::os::unix::net::UnixListener::bind(format!("{runtime}/riced/{two_id}.sock")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_CONFIG_HOME", Path::new(&runtime).join("configs"))
        .args(["ipc", "-c", "two", "settings", "x"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let entry = std::fs::read_dir(format!("{runtime}/riced/ipc/{two_id}"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(entry.path()).unwrap()).unwrap();
    assert_eq!(value["command"], "settings");

    let ambiguous = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env("XDG_RUNTIME_DIR", &runtime)
        .args(["ipc", "settings"])
        .output()
        .unwrap();
    assert!(!ambiguous.status.success());
    assert!(
        String::from_utf8_lossy(&ambiguous.stderr).contains("expected one running profile"),
        "{}",
        String::from_utf8_lossy(&ambiguous.stderr)
    );
    let _ = std::fs::remove_dir_all(&runtime);
}

#[test]
fn help_does_not_require_configuration_or_compositor() {
    let output = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--config <NAME>"), "{help}");
    assert!(help.contains("--path <DIRECTORY>"), "{help}");
    assert!(help.contains("--logs"), "{help}");
    assert!(help.contains("--log-retention"), "{help}");
    let logs = Command::new(env!("CARGO_BIN_EXE_riced"))
        .args(["logs", "--help"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&logs.stdout).contains("--clear"));
}

#[test]
fn logs_are_hidden_by_default_and_available_on_request() {
    let directory =
        std::path::PathBuf::from(format!("/tmp/opencode/riced-logs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let profile = directory.join("riced/test");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(
        profile.join("shell.lua"),
        r#"log("stored diagnostic\nsecond line\n\nlast line"); error("logs test complete")"#,
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_riced"))
            .env("XDG_CONFIG_HOME", &directory)
            .env("XDG_RUNTIME_DIR", &directory)
            .args(args)
            .output()
            .unwrap()
    };

    let quiet = run(&["-c", "test"]);
    assert!(!quiet.status.success());
    assert!(!String::from_utf8_lossy(&quiet.stderr).contains("[lua]: stored diagnostic"));

    let mirrored = run(&["--logs", "-c", "test"]);
    assert!(!mirrored.status.success());
    assert!(String::from_utf8_lossy(&mirrored.stderr).contains("[lua]: stored diagnostic"));

    let stored = run(&["logs", "-c", "test"]);
    assert!(stored.status.success());
    assert_eq!(
        String::from_utf8_lossy(&stored.stdout),
        "[lua]: stored diagnostic\nsecond line\n\nlast line\n"
    );

    let cleared = run(&["logs", "-c", "test", "--clear"]);
    assert!(cleared.status.success());
    let empty = run(&["logs", "-c", "test"]);
    assert!(empty.status.success());
    assert!(empty.stdout.is_empty());

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn rejected_duplicate_start_preserves_running_logs() {
    let directory = std::path::PathBuf::from(format!(
        "/tmp/opencode/riced-duplicate-logs-{}",
        std::process::id()
    ));
    let profile = directory.join("profile");
    let logs = directory.join("riced/logs");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(profile.join("shell.lua"), "error('must not load')").unwrap();
    let id = profile_id(&profile);
    let _listener =
        std::os::unix::net::UnixListener::bind(directory.join(format!("riced/{id}.sock"))).unwrap();
    let path = logs.join(format!("{id}.log"));
    let contents = "123 [lua]: active daemon diagnostic\n";
    std::fs::write(&path, contents).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env("XDG_RUNTIME_DIR", &directory)
        .arg("-p")
        .arg(&profile)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already in use"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn clearing_logs_waits_for_an_in_progress_prune() {
    let directory = std::path::PathBuf::from(format!(
        "/tmp/opencode/riced-clear-lock-{}",
        std::process::id()
    ));
    let profile = directory.join("profile");
    let logs = directory.join("riced/logs");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::create_dir_all(&logs).unwrap();
    let path = logs.join(format!("{}.log", profile_id(&profile)));
    std::fs::write(&path, "1 expired\n2 retained\n").unwrap();
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.lock().unwrap();

    let mut clear = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env("XDG_RUNTIME_DIR", &directory)
        .args(["logs", "--clear", "-p"])
        .arg(&profile)
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let waiting = clear.try_wait().unwrap().is_none();
    // Finish the daemon's locked rewrite after the CLI has requested a clear.
    file.set_len(0).unwrap();
    std::io::Write::write_all(&mut file, b"2 retained\n").unwrap();
    file.unlock().unwrap();
    assert!(clear.wait().unwrap().success());
    assert!(waiting, "clear finished before pruning released its lock");
    assert!(std::fs::read(&path).unwrap().is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn default_named_and_explicit_directories_load_their_own_shells() {
    let root = std::path::PathBuf::from(format!(
        "/tmp/opencode/riced-cli-paths-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for (directory, marker) in [
        ("xdg/riced", "default config loaded"),
        ("xdg/riced/test", "named config loaded"),
        ("home/.config/riced", "home config loaded"),
        ("custom config", "explicit config loaded"),
    ] {
        let directory = root.join(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("shell.lua"), "error((require('marker')))").unwrap();
        std::fs::write(directory.join("marker.lua"), format!("return '{marker}'")).unwrap();
    }
    for (args, xdg, marker) in [
        (vec![], Some(root.join("xdg")), "default config loaded"),
        (
            vec!["-c", "test"],
            Some(root.join("xdg")),
            "named config loaded",
        ),
        (vec![], None, "home config loaded"),
        (
            vec![],
            Some("relative-xdg-is-ignored".into()),
            "home config loaded",
        ),
        (vec!["-p", "custom config"], None, "explicit config loaded"),
        (
            vec!["--path", "custom config/../custom config"],
            None,
            "explicit config loaded",
        ),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_riced"));
        command
            .current_dir(&root)
            .env("HOME", root.join("home"))
            .env("XDG_RUNTIME_DIR", &root)
            .env_remove("XDG_CONFIG_HOME")
            .args(args);
        if let Some(xdg) = xdg {
            command.env("XDG_CONFIG_HOME", xdg);
        }
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success());
        assert!(stderr.contains(marker), "{stderr}");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env("XDG_RUNTIME_DIR", &root)
        .arg("-p")
        .arg(root.join("custom config"))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("explicit config loaded"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn extension_build_requires_an_output_or_running_profile() {
    for args in [
        vec!["extension", "build"],
        vec!["extension", "build", "./path"],
        vec!["extension", "build", "-o", "./output"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_riced"))
            .env_remove("XDG_RUNTIME_DIR")
            .args(&args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{args:?}: {output:?}");
    }
    let missing_output = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env_remove("XDG_RUNTIME_DIR")
        .args(["extension", "build", "./path"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&missing_output.stderr).contains("-o"),
        "{}",
        String::from_utf8_lossy(&missing_output.stderr)
    );
}

#[test]
fn extension_build_rejects_invalid_arguments() {
    for args in [
        vec!["extension", "build", "-c", "test", "-p", "/other", "./path"],
        vec!["extension", "build", "./path", "-o", ""],
        vec!["extension", "build", "", "-o", "./output"],
        vec!["extension", "build", "--unknown"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_riced"))
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn clap_rejects_invalid_selectors_and_reports_version() {
    for args in [
        vec!["-c", "test", "-p", "/other"],
        vec!["-c", "../test"],
        vec!["-c", ""],
        vec!["-c"],
        vec!["-p"],
        vec!["--unknown"],
        vec!["ipc", "-c", "test", "-p", "/other", "settings"],
        vec!["ipc"],
        vec!["logs", "-c", "test", "-p", "/other"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_riced"))
            .env_remove("HOME")
            .env_remove("XDG_CONFIG_HOME")
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let empty = Command::new(env!("CARGO_BIN_EXE_riced"))
        .args(["ipc", ""])
        .output()
        .unwrap();
    assert!(!empty.status.success());
    assert!(String::from_utf8_lossy(&empty.stderr).contains("IPC command must not be empty"));
    let output = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("riced ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn ipc_paths_distinguish_equal_basenames_and_follow_aliases() {
    let root = std::path::PathBuf::from(format!(
        "/tmp/opencode/riced-ipc-paths-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("riced")).unwrap();
    let one = root.join("one/config");
    let two = root.join("two/config");
    std::fs::create_dir_all(&one).unwrap();
    std::fs::create_dir_all(&two).unwrap();
    std::os::unix::fs::symlink(&two, root.join("alias")).unwrap();
    let one_id = profile_id(&one);
    let two_id = profile_id(&two);
    let _one =
        std::os::unix::net::UnixListener::bind(root.join(format!("riced/{one_id}.sock"))).unwrap();
    let _two =
        std::os::unix::net::UnixListener::bind(root.join(format!("riced/{two_id}.sock"))).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_riced"))
        .current_dir(&root)
        .env("XDG_RUNTIME_DIR", &root)
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .args([
            "ipc", "-p", "alias", "settings", "-c", "payload", "--help", "",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let entry = std::fs::read_dir(root.join(format!("riced/ipc/{two_id}")))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
    assert_eq!(
        value["args"],
        serde_json::json!(["-c", "payload", "--help", ""])
    );
    assert!(!root.join(format!("riced/ipc/{one_id}")).exists());
    std::fs::remove_dir_all(root).unwrap();
}

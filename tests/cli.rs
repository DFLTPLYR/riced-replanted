use std::process::Command;

#[test]
fn missing_profile_reports_shell_path_and_fails() {
    let config = format!("/tmp/opencode/riced-missing-config-{}", std::process::id());
    let output = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env("XDG_CONFIG_HOME", &config)
        .args(["-p", "test"])
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
        .args(["-p", "test"])
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
    let _one = std::os::unix::net::UnixListener::bind(format!("{runtime}/riced/one.sock")).unwrap();
    let _two = std::os::unix::net::UnixListener::bind(format!("{runtime}/riced/two.sock")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_riced"))
        .env("XDG_RUNTIME_DIR", &runtime)
        .args(["ipc", "-p", "two", "settings", "x"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let entry = std::fs::read_dir(format!("{runtime}/riced/ipc/two"))
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("riced -p <profile>"));
}

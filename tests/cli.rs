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
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains(&format!("cannot read {config}/riced/test/shell.lua"))
    );
}

#[test]
fn ipc_queues_command_with_arguments() {
    let runtime = format!("/tmp/opencode/riced-ipc-{}", std::process::id());
    let _ = std::fs::remove_dir_all(&runtime);
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

    let entry = std::fs::read_dir(format!("{runtime}/riced/ipc"))
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

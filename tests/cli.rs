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

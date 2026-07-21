use std::process::Command;

#[test]
fn help_describes_the_primary_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_silo"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for command in ["init", "shell", "otp-check", "import", "export"] {
        assert!(help.contains(command), "missing {command} in help output");
    }
}

#[test]
fn generated_password_respects_requested_length() {
    let output = Command::new(env!("CARGO_BIN_EXE_silo"))
        .args(["generate", "--length", "24"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim().len(), 24);
}

#[test]
fn add_documents_non_interactive_value_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_silo"))
        .args(["add", "example", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for flag in [
        "--url",
        "--username",
        "--email",
        "--password",
        "--password-file",
        "--totp-secret",
    ] {
        assert!(help.contains(flag), "missing {flag} in add help output");
    }
}

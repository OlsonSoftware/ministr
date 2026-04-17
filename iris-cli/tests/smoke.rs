//! Smoke tests for the iris CLI binary.
//!
//! Verifies the binary builds, runs, and responds to basic CLI flags.

use std::process::Command;

/// The iris binary should exit 0 with `--help`.
#[test]
fn help_flag_exits_successfully() {
    let output = Command::new(env!("CARGO_BIN_EXE_iris"))
        .arg("--help")
        .output()
        .expect("failed to execute iris binary");

    assert!(
        output.status.success(),
        "iris --help should exit 0, got: {}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("context cache") || stdout.contains("iris") || stdout.contains("corpus"),
        "help output should mention iris functionality, got: {stdout}"
    );
}

/// The iris binary should exit 0 with `--version`.
#[test]
fn version_flag_exits_successfully() {
    let output = Command::new(env!("CARGO_BIN_EXE_iris"))
        .arg("--version")
        .output()
        .expect("failed to execute iris binary");

    assert!(
        output.status.success(),
        "iris --version should exit 0, got: {}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0.1.0"),
        "version output should contain 0.1.0, got: {stdout}"
    );
}

//! Smoke tests for the ministr CLI binary.
//!
//! Verifies the binary builds, runs, and responds to basic CLI flags.

use std::process::Command;

/// The ministr binary should exit 0 with `--help`.
#[test]
fn help_flag_exits_successfully() {
    let output = Command::new(env!("CARGO_BIN_EXE_ministr"))
        .arg("--help")
        .output()
        .expect("failed to execute ministr binary");

    assert!(
        output.status.success(),
        "ministr --help should exit 0, got: {}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("code intelligence") || stdout.contains("ministr") || stdout.contains("corpus"),
        "help output should mention ministr functionality, got: {stdout}"
    );
}

/// The ministr binary should exit 0 with `--version`.
#[test]
fn version_flag_exits_successfully() {
    let output = Command::new(env!("CARGO_BIN_EXE_ministr"))
        .arg("--version")
        .output()
        .expect("failed to execute ministr binary");

    assert!(
        output.status.success(),
        "ministr --version should exit 0, got: {}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let want = env!("CARGO_PKG_VERSION");
    assert!(
        stdout.contains(want),
        "version output should contain {want}, got: {stdout}"
    );
}

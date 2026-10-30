//! Smoke tests: the built binary starts, reports its version, and rejects bad input.

use std::process::{Command, Output};

fn molt(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_molt"))
        .args(args)
        .output()
        .expect("failed to run molt binary")
}

#[test]
fn version_matches_cargo_manifest() {
    let out = molt(&["--version"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("molt {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_exits_zero() {
    let out = molt(&["--help"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Usage: molt"));
}

#[test]
fn unknown_flag_is_rejected() {
    let out = molt(&["--definitely-not-a-flag"]);
    assert_eq!(out.status.code(), Some(2));
}

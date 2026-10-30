//! Tests for P-08 process detection (specs/006-clean.md CN-03).

use super::*;

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn cn_03_running_programs_are_found_by_short_and_long_names() {
    let dir = TempDir::new().unwrap();
    // Longer than the 15 bytes the kernel keeps in `comm`.
    let long = dir.path().join("molt-test-program-with-a-long-name");
    fs::copy("/usr/bin/sleep", &long).unwrap();
    fs::set_permissions(&long, fs::Permissions::from_mode(0o755)).unwrap();
    let mut child = Command::new(&long).arg("30").spawn().unwrap();

    let names = running();
    child.kill().unwrap();
    child.wait().unwrap();

    assert!(names.contains("molt-test-program-with-a-long-name"));
    assert!(names.contains("molt-test-progr"), "comm is cut to 15 bytes");
    assert!(!running().contains("molt-test-program-with-a-long-name"));
}

#[test]
fn cn_03_cmdline_parsing() {
    assert_eq!(
        program(b"/usr/bin/firefox\0-P\0default\0"),
        Some("firefox".to_owned())
    );
    assert_eq!(program(b"slack\0"), Some("slack".to_owned()));
    assert_eq!(program(b""), None);
    assert_eq!(program(b"\0"), None);
}

//! Tests for P-08 process detection (specs/006-clean.md CN-03).

use super::*;

use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

use tempfile::TempDir;

#[test]
fn cn_03_running_programs_are_found_by_short_and_long_names() {
    let dir = TempDir::new().unwrap();
    // Longer than the 15 bytes the kernel keeps in `comm`.
    let long = dir.path().join("molt-test-program-with-a-long-name");
    // A copy of `sh` blocked on a builtin: a multicall `sleep` would exit at once under
    // an unknown name, leaving a zombie with an empty cmdline.
    fs::copy("/bin/sh", &long).unwrap();
    fs::set_permissions(&long, fs::Permissions::from_mode(0o755)).unwrap();
    let mut child = spawn_fresh_copy(Command::new(&long).args(["-c", "read line"]));
    wait_for_exec(child.id(), "molt-test-progr");

    let names = running();
    child.kill().unwrap();
    child.wait().unwrap();

    assert!(names.contains("molt-test-program-with-a-long-name"));
    assert!(names.contains("molt-test-progr"), "comm is cut to 15 bytes");
    assert!(!running().contains("molt-test-program-with-a-long-name"));
}

/// Spawns a just-copied binary. Another test thread's fork may briefly hold the copy's
/// write handle, and exec then fails with ETXTBSY, so retry that.
fn spawn_fresh_copy(command: &mut Command) -> std::process::Child {
    for _ in 0..100 {
        match command.stdin(Stdio::piped()).spawn() {
            Err(e) if e.raw_os_error() == Some(rustix::io::Errno::TXTBSY.raw_os_error()) => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            spawned => return spawned.unwrap(),
        }
    }
    panic!("the copied binary stayed busy")
}

/// Waits until process `pid` has exec'd: until then, `/proc` still shows the parent.
fn wait_for_exec(pid: u32, comm: &str) {
    let path = format!("/proc/{pid}/comm");
    for _ in 0..500 {
        if fs::read_to_string(&path).is_ok_and(|c| c.trim_end() == comm) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("process {pid} never became {comm}");
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

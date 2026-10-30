//! Tests for the menu's keys, moves, entries and screen (MN-02, MN-03).

use super::*;

#[test]
fn mn_02_keys_are_arrows_jk_enter_and_quit() {
    assert_eq!(
        keys(b"jk\x1b[B\x1b[A\x1bOB\r\nq\x03"),
        [
            Key::Down,
            Key::Up,
            Key::Down,
            Key::Up,
            Key::Down,
            Key::Run,
            Key::Run,
            Key::Quit,
            Key::Quit
        ]
    );
    assert_eq!(keys(b"\x1b"), [Key::Quit], "a lone Esc");
    assert!(
        keys(b"\x1b[Zx").is_empty(),
        "unknown sequences and keys do nothing"
    );
}

#[test]
fn mn_02_moves_stop_at_the_ends() {
    assert_eq!(step(0, Key::Up), 0);
    assert_eq!(step(0, Key::Down), 1);
    let last = Entry::ALL.len() - 1;
    assert_eq!(step(last, Key::Down), last);
    assert_eq!(Entry::ALL[last], Entry::Quit);
}

#[test]
fn mn_03_each_entry_runs_its_own_command() {
    let args: Vec<&[&str]> = Entry::ALL.iter().map(|entry| entry.args()).collect();
    assert_eq!(
        args,
        [
            &["scan"][..],
            &["clean"],
            &["purge"],
            &["installer"],
            &["uninstall"],
            &["analyze"],
            &["status", "--watch"],
            &["system"],
            &["optimize"],
            &["history"],
            &[],
        ]
    );
}

#[test]
fn mn_02_the_screen_fits_and_marks_the_choice() {
    let banner = ["ART", "tagline"];
    let lines = lines(&banner, 1, 80, 30);
    assert_eq!(lines[0], "ART");
    let clean = lines
        .iter()
        .position(|line| line.contains("Clean"))
        .unwrap();
    assert!(lines[clean].starts_with("> Clean"), "{lines:?}");
    assert!(lines[clean - 1].starts_with("  Scan"), "{lines:?}");
    assert!(lines.last().unwrap().contains("Enter run"));

    let small = super::lines(&banner, 0, 30, 14);
    assert!(
        !small.contains(&"ART".to_owned()),
        "no room for the banner: {small:?}"
    );
    assert!(
        small.iter().all(|line| line.chars().count() <= 30),
        "{small:?}"
    );
}

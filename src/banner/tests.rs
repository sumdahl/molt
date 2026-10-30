//! Tests for the banner's shape and colour (BN-01, BN-04).

use super::*;

#[test]
fn bn_01_six_printable_ascii_lines_at_most_40_wide_then_a_blank_line() {
    let plain = banner(Colour::Off);
    let lines: Vec<&str> = plain.lines().collect();
    assert_eq!(lines.len(), 7, "{plain}");
    assert_eq!(lines[6], "");
    for line in &lines[..6] {
        assert!(line.len() <= 40, "{line:?}");
        assert!(line.bytes().all(|b| (b' '..=b'~').contains(&b)), "{line:?}");
    }
    assert!(lines[5].contains("a careful disk cleaner for Linux"));
}

#[test]
fn bn_04_green_art_dim_tagline_reset_or_plain() {
    let coloured = banner(Colour::On);
    assert!(coloured.starts_with("\x1b[32m"), "{coloured:?}");
    assert!(
        coloured.contains("\x1b[0m\x1b[2m  a careful"),
        "{coloured:?}"
    );
    assert!(coloured.trim_end().ends_with("\x1b[0m"), "{coloured:?}");
    assert!(!banner(Colour::Off).contains('\x1b'));
}

#[test]
fn bn_04_no_color_and_dumb_terminals_get_plain_text() {
    assert_eq!(Colour::from_env(None, Some("xterm")), Colour::On);
    assert_eq!(Colour::from_env(Some(""), Some("xterm")), Colour::On);
    assert_eq!(Colour::from_env(Some("1"), Some("xterm")), Colour::Off);
    assert_eq!(Colour::from_env(None, Some("dumb")), Colour::Off);
}

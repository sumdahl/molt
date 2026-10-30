//! Tests for the live view's keys and screen (ST-15 to ST-17), without a terminal.

use super::*;

#[test]
fn st_17_keys_are_quit_pause_and_refresh() {
    assert_eq!(
        keys(b"qp\x03rx"),
        [Key::Quit, Key::Pause, Key::Quit, Key::Refresh]
    );
}

#[test]
fn st_15_st_16_the_frame_fits_and_keeps_the_footer() {
    let report = "CPU\n  12 CPUs, 3% busy, load 1.04 0.88 1.02\n\nMemory\n  lots\n";
    let lines = frame(report, &footer(Duration::from_secs(2), false), 20, 4);
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "CPU");
    assert_eq!(lines[1], "  12 CPUs, 3% busy, ", "cut to 20 characters");
    assert!(lines[3].starts_with("every 2s"), "{lines:?}");

    let short = frame("CPU\n", &footer(Duration::from_secs(5), true), 40, 4);
    assert_eq!(short.len(), 4, "padded to the height");
    assert!(short[3].starts_with("paused"), "{short:?}");
}

//! `onyx status --watch` (specs/013-status.md ST-15 to ST-19): the report full screen,
//! refreshed, with rates over each whole period. Reads only.

use std::error::Error;
use std::time::{Duration, Instant};

use super::{Sample, disks, report, status};
use crate::terminal::Screen;

#[cfg(test)]
mod tests;

/// ST-17
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Quit,
    Pause,
    Refresh,
}

fn keys(bytes: &[u8]) -> Vec<Key> {
    bytes
        .iter()
        .filter_map(|byte| match byte {
            // Raw mode turns Ctrl-C into a byte.
            b'q' | 0x03 => Some(Key::Quit),
            b'p' => Some(Key::Pause),
            b'r' => Some(Key::Refresh),
            _ => None,
        })
        .collect()
}

/// ST-15, ST-16: the report's lines cut to `width`, as many as fit above the footer.
fn frame(report: &str, footer: &str, width: usize, height: usize) -> Vec<String> {
    let cut = |line: &str| line.chars().take(width).collect::<String>();
    let mut lines: Vec<String> = report
        .lines()
        .take(height.saturating_sub(1))
        .map(cut)
        .collect();
    while lines.len() < height.saturating_sub(1) {
        lines.push(String::new());
    }
    lines.push(cut(footer));
    lines
}

fn footer(every: Duration, paused: bool) -> String {
    let state = if paused {
        "paused".to_owned()
    } else {
        format!("every {}s", every.as_secs())
    };
    format!("{state}  ·  p pause  r refresh  q quit")
}

/// ST-15 to ST-19: refreshes the report every `every` until `q` or Ctrl-C.
pub fn watch(every: Duration) -> Result<(), Box<dyn Error>> {
    let screen = Screen::enter()?;
    let mut shown = status().to_string();
    let mut before = Sample::now();
    let mut paused = false;
    loop {
        let next = Instant::now() + every;
        let mut refresh = false;
        // AN-16, ST-16: redraw on every read, so a resize shows within 200 ms.
        while !refresh && (paused || Instant::now() < next) {
            let (width, height) = screen.size();
            screen.draw(&frame(&shown, &footer(every, paused), width, height))?;
            for key in keys(&screen.read_bytes()?) {
                match key {
                    Key::Quit => return Ok(()),
                    Key::Pause => paused = !paused,
                    Key::Refresh => refresh = true,
                }
            }
        }
        // ST-15: the rates are over the whole time since the last report.
        let after = Sample::now();
        shown = report(&before, &after, disks()).to_string();
        before = after;
    }
}

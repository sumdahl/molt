//! The browser on a real terminal (specs/012-analyze.md AN-10 to AN-17): raw mode through
//! `rustix` termios and ANSI escapes, written here, with no terminal crate (012 Decisions).

use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::Ordering;

use rustix::termios::{self, OptionalActions, SpecialCodeIndex, Termios};

use super::browse::{Browser, Effect, Key, keys};
use super::{AnalyzeError, Node, Progress, files, measure_folders};
use crate::scan::human_size;

/// AN-10: browses `root` until the user quits. Changes nothing (AN-14).
pub fn browse(root: &Path) -> Result<(), Box<dyn Error>> {
    // AN-05, before the screen changes, so the error prints as usual.
    let meta =
        fs::symlink_metadata(root).map_err(|e| AnalyzeError::Unreadable(root.to_owned(), e))?;
    if !meta.is_dir() {
        return Err(AnalyzeError::NotADirectory(root.to_owned()).into());
    }
    let dev = meta.dev();
    let screen = Screen::enter()?;
    let mut queued = Vec::new();
    let Some(tree) = screen.measure(root, &mut queued)? else {
        return Ok(());
    };
    let mut browser = Browser::new(root.to_owned(), tree, files(root, dev));
    let mut size = (0, 0);
    loop {
        // AN-16: checked on every key, and between reads while idle.
        let now = screen.size();
        if now != size || !queued.is_empty() {
            size = now;
            browser.set_rows(size.1.saturating_sub(2));
        }
        for key in std::mem::take(&mut queued) {
            match browser.press(key) {
                Effect::Redraw => {}
                Effect::Quit => return Ok(()),
                Effect::Load(path) => browser.show(files(&path, dev)),
                Effect::Measure(path) => {
                    let Some(fresh) = screen.measure(&path, &mut queued)? else {
                        return Ok(());
                    };
                    browser.replace(fresh, files(&path, dev));
                }
            }
        }
        screen.draw(&browser.lines(size.0, size.1))?;
        queued = screen.read_keys()?;
    }
}

/// The terminal in raw mode on the alternate screen. Dropping it puts everything back,
/// whether molt leaves by `q`, an error or a panic (AN-15).
struct Screen {
    saved: Termios,
}

impl Screen {
    fn enter() -> io::Result<Self> {
        let stdin = io::stdin();
        let saved = termios::tcgetattr(stdin.as_fd())?;
        let mut raw = saved.clone();
        raw.make_raw();
        // A read returns after 200 ms with nothing, so the screen keeps up while idle.
        raw.special_codes[SpecialCodeIndex::VMIN] = 0;
        raw.special_codes[SpecialCodeIndex::VTIME] = 2;
        termios::tcsetattr(stdin.as_fd(), OptionalActions::Now, &raw)?;
        let screen = Self { saved };
        // The alternate screen, and no cursor.
        write_out("\x1b[?1049h\x1b[?25l")?;
        Ok(screen)
    }

    /// Columns and rows, or 80 × 24 if the terminal won't say (a 0 × 0 pty).
    fn size(&self) -> (usize, usize) {
        match termios::tcgetwinsize(io::stdout().as_fd()) {
            Ok(size) if size.ws_col > 0 && size.ws_row > 0 => (
                usize::from(size.ws_col.max(20)),
                usize::from(size.ws_row.max(5)),
            ),
            _ => (80, 24),
        }
    }

    fn read_keys(&self) -> io::Result<Vec<Key>> {
        let mut buf = [0u8; 64];
        // ponytail: an escape sequence split across two reads is dropped; one more key
        // press fixes it.
        match rustix::io::read(io::stdin().as_fd(), &mut buf) {
            Ok(len) => Ok(keys(&buf[..len])),
            Err(rustix::io::Errno::INTR) => Ok(Vec::new()),
            Err(errno) => Err(errno.into()),
        }
    }

    fn draw(&self, lines: &[String]) -> io::Result<()> {
        let mut frame = String::from("\x1b[H");
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                frame.push_str("\r\n");
            }
            frame.push_str(line);
            frame.push_str("\x1b[K");
        }
        frame.push_str("\x1b[J");
        write_out(&frame)
    }

    /// AN-11: measures `path` on another thread, showing progress. `q` or Ctrl-C stops it
    /// and gives `None`; other keys pressed meanwhile are kept in `queued`.
    fn measure(&self, path: &Path, queued: &mut Vec<Key>) -> Result<Option<Node>, Box<dyn Error>> {
        let progress = Progress::default();
        std::thread::scope(|scope| {
            let walk = scope.spawn(|| measure_folders(path, &progress));
            while !walk.is_finished() {
                let line = format!(
                    "Measuring {} …  {} folders, {}   (q to stop)",
                    path.display(),
                    progress.folders.load(Ordering::Relaxed),
                    human_size(progress.bytes.load(Ordering::Relaxed))
                );
                self.draw(&[line])?;
                for key in self.read_keys()? {
                    if key == Key::Quit {
                        progress.stop.store(true, Ordering::Relaxed);
                    } else {
                        queued.push(key);
                    }
                }
            }
            let tree = walk
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
            Ok((!progress.stop.load(Ordering::Relaxed)).then_some(tree))
        })
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        // Best effort: there is nowhere left to report a failure to.
        let _ = termios::tcsetattr(io::stdin().as_fd(), OptionalActions::Now, &self.saved);
        let _ = write_out("\x1b[?25h\x1b[?1049l");
    }
}

fn write_out(text: &str) -> io::Result<()> {
    let mut out = io::stdout().lock();
    out.write_all(text.as_bytes())?;
    out.flush()
}

//! The browser on a real terminal (specs/012-analyze.md AN-10 to AN-17): raw mode through
//! `rustix` termios and ANSI escapes, written here, with no terminal crate (012 Decisions).

use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use rustix::termios::{self, OptionalActions, SpecialCodeIndex, Termios};

use super::browse::{Browser, Effect, Key, keys};
use super::{AnalyzeError, Node, Progress, files, measure_folders};
use crate::safe_delete::{
    self, Candidate, Context, Execution, Mode, Outcome, Permissions, Plan, Privilege,
};
use crate::scan::{Item, human_size};

/// AN-10: browses `root` until the user quits. It changes nothing except through the
/// trash step (AN-18 to AN-24), which uses safe delete with `ctx`. `dry_run` names why
/// the trash step may only preview (P-05), if it may.
pub fn browse(root: &Path, ctx: &Context, dry_run: Option<&str>) -> Result<(), Box<dyn Error>> {
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
                Effect::Mark(path) => {
                    // AN-19: an entry gone since the screen was drawn just isn't marked.
                    if let Ok(candidate) = Candidate::scan(&path) {
                        browser.mark(candidate);
                    }
                }
                Effect::Trash => {
                    let picked = browser.picked();
                    let Some(trashed) = screen.trash(&picked, ctx, dry_run, size)? else {
                        continue;
                    };
                    // AN-22: measure again the deepest folder holding what moved.
                    browser.clear_marks();
                    let trail = browser.common_trail(&trashed);
                    let path = browser.path_of(&trail);
                    let Some(fresh) = screen.measure(&path, &mut queued)? else {
                        return Ok(());
                    };
                    browser.replace_at(&trail, fresh);
                    browser.show(files(&browser.path(), dev));
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
        // ponytail: an escape sequence split across two reads is dropped; one more key
        // press fixes it.
        self.read_bytes().map(|bytes| keys(&bytes))
    }

    fn read_bytes(&self) -> io::Result<Vec<u8>> {
        let mut buf = [0u8; 64];
        match rustix::io::read(io::stdin().as_fd(), &mut buf) {
            Ok(len) => Ok(buf[..len].to_vec()),
            Err(rustix::io::Errno::INTR) => Ok(Vec::new()),
            Err(errno) => Err(errno.into()),
        }
    }

    /// Waits for a key and gives its first byte.
    fn wait_byte(&self) -> io::Result<u8> {
        loop {
            if let Some(&first) = self.read_bytes()?.first() {
                return Ok(first);
            }
        }
    }

    /// AN-20 to AN-24: the dry run of exactly `picked` on screen, one question, then one
    /// live plan, always in trash mode. Gives the paths that moved, or `None` if nothing
    /// was done.
    fn trash(
        &self,
        picked: &[Candidate],
        ctx: &Context,
        dry_run: Option<&str>,
        (width, height): (usize, usize),
    ) -> Result<Option<Vec<PathBuf>>, Box<dyn Error>> {
        if picked.is_empty() {
            return Ok(None);
        }
        let plan = |execution| Plan {
            candidates: picked.to_vec(),
            // AN-23: the only mode the browser has.
            mode: Mode::Trash,
            execution,
            privilege: Privilege::User,
            permissions: Permissions::Keep,
        };
        let preview = match safe_delete::execute(&plan(Execution::DryRun), ctx) {
            Ok(report) => report.results,
            Err(refused) => {
                return self.notice(&format!("Nothing moved: {refused}."), width, height);
            }
        };
        let items: Vec<Item> = preview
            .iter()
            .map(|(path, outcome)| Item::new(path, *outcome))
            .collect();
        let would: Vec<&Item> = items
            .iter()
            .filter(|item| item.outcome == "would-trash")
            .collect();
        let bytes: u64 = would.iter().map(|item| item.bytes).sum();
        let mut lines = vec![
            "Preview. Nothing has changed yet:".to_owned(),
            String::new(),
        ];
        lines.extend(items.iter().map(|item| outcome_line(item, width)));
        lines.push(String::new());
        // P-05: a dry run shows the preview and stops.
        if let Some(why) = dry_run {
            lines.push(format!("{why} Press any key."));
            self.draw(&fit_screen(lines, height))?;
            self.wait_byte()?;
            return Ok(None);
        }
        if would.is_empty() {
            lines.push("Nothing here can be moved to the trash. Press any key.".to_owned());
            self.draw(&fit_screen(lines, height))?;
            self.wait_byte()?;
            return Ok(None);
        }
        lines.push(format!(
            "Move {} items ({}) to the trash? [y/N]",
            would.len(),
            human_size(bytes)
        ));
        self.draw(&fit_screen(lines, height))?;
        // AN-21: only `y` goes ahead.
        if !matches!(self.wait_byte()?, b'y' | b'Y') {
            return Ok(None);
        }
        let report = match safe_delete::execute(&plan(Execution::Live), ctx) {
            Ok(report) => report.results,
            // AN-24: the log couldn't be opened, so nothing moved.
            Err(refused) => {
                return self.notice(&format!("Nothing moved: {refused}."), width, height);
            }
        };
        let items: Vec<Item> = report
            .iter()
            .map(|(path, outcome)| Item::new(path, *outcome))
            .collect();
        let mut lines = vec!["Done:".to_owned(), String::new()];
        lines.extend(items.iter().map(|item| outcome_line(item, width)));
        lines.push(String::new());
        lines.push("Press any key.".to_owned());
        self.draw(&fit_screen(lines, height))?;
        self.wait_byte()?;
        Ok(Some(
            report
                .into_iter()
                .filter(|(_, outcome)| matches!(outcome, Outcome::Trashed { .. }))
                .map(|(path, _)| path)
                .collect(),
        ))
    }

    /// One message, until a key is pressed.
    fn notice(
        &self,
        text: &str,
        width: usize,
        height: usize,
    ) -> Result<Option<Vec<PathBuf>>, Box<dyn Error>> {
        let line: String = text.chars().take(width).collect();
        self.draw(&fit_screen(vec![line, "Press any key.".to_owned()], height))?;
        self.wait_byte()?;
        Ok(None)
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

/// AN-20, AN-22: an item's size, what happens to it, and why, cut to `width`.
fn outcome_line(item: &Item, width: usize) -> String {
    let reason = item
        .reason
        .as_deref()
        .map(|reason| format!(" ({reason})"))
        .unwrap_or_default();
    let line = format!(
        "{:>10}  {:<12} {}{reason}",
        human_size(item.bytes),
        item.outcome,
        item.path
    );
    line.chars().take(width).collect()
}

/// Keeps the last line (the question) on screen when the list is too long.
fn fit_screen(mut lines: Vec<String>, height: usize) -> Vec<String> {
    if lines.len() > height {
        let last = lines.split_off(lines.len() - 1);
        let more = lines.len() + 2 - height;
        lines.truncate(height.saturating_sub(2));
        lines.push(format!("… {more} more lines"));
        lines.extend(last);
    }
    lines
}

fn write_out(text: &str) -> io::Result<()> {
    let mut out = io::stdout().lock();
    out.write_all(text.as_bytes())?;
    out.flush()
}

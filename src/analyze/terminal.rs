//! The browser on a real terminal (specs/012-analyze.md AN-10 to AN-24), on the shared
//! `crate::terminal::Screen`.

use std::error::Error;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::browse::{Browser, Effect, Key, keys};
use super::{AnalyzeError, Node, Progress, files, measure_folders};
use crate::safe_delete::{
    self, Candidate, Context, Execution, Mode, Outcome, Permissions, Plan, Privilege,
};
use crate::scan::{Item, human_size};
use crate::terminal::Screen;

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
    let Some(tree) = measure(&screen, root, &mut queued)? else {
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
                    let Some(fresh) = measure(&screen, &path, &mut queued)? else {
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
                    let Some(trashed) = trash(&screen, &picked, ctx, dry_run, size)? else {
                        continue;
                    };
                    // AN-22: measure again the deepest folder holding what moved.
                    browser.clear_marks();
                    let trail = browser.common_trail(&trashed);
                    let path = browser.path_of(&trail);
                    let Some(fresh) = measure(&screen, &path, &mut queued)? else {
                        return Ok(());
                    };
                    browser.replace_at(&trail, fresh);
                    browser.show(files(&browser.path(), dev));
                }
            }
        }
        screen.draw(&browser.lines(size.0, size.1))?;
        queued = read_keys(&screen)?;
    }
}

/// Keys typed, decoded for the browser.
// ponytail: an escape sequence split across two reads is dropped; one more key press
// fixes it.
fn read_keys(screen: &Screen) -> io::Result<Vec<Key>> {
    screen.read_bytes().map(|bytes| keys(&bytes))
}

/// AN-20 to AN-24: the dry run of exactly `picked` on screen, one question, then one
/// live plan, always in trash mode. Gives the paths that moved, or `None` if nothing
/// was done.
fn trash(
    screen: &Screen,
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
            return notice(screen, &format!("Nothing moved: {refused}."), width, height);
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
        screen.draw(&fit_screen(lines, height))?;
        screen.wait_byte()?;
        return Ok(None);
    }
    if would.is_empty() {
        lines.push("Nothing here can be moved to the trash. Press any key.".to_owned());
        screen.draw(&fit_screen(lines, height))?;
        screen.wait_byte()?;
        return Ok(None);
    }
    lines.push(format!(
        "Move {} items ({}) to the trash? [y/N]",
        would.len(),
        human_size(bytes)
    ));
    screen.draw(&fit_screen(lines, height))?;
    // AN-21: only `y` goes ahead.
    if !matches!(screen.wait_byte()?, b'y' | b'Y') {
        return Ok(None);
    }
    let report = match safe_delete::execute(&plan(Execution::Live), ctx) {
        Ok(report) => report.results,
        // AN-24: the log couldn't be opened, so nothing moved.
        Err(refused) => {
            return notice(screen, &format!("Nothing moved: {refused}."), width, height);
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
    screen.draw(&fit_screen(lines, height))?;
    screen.wait_byte()?;
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
    screen: &Screen,
    text: &str,
    width: usize,
    height: usize,
) -> Result<Option<Vec<PathBuf>>, Box<dyn Error>> {
    let line: String = text.chars().take(width).collect();
    screen.draw(&fit_screen(vec![line, "Press any key.".to_owned()], height))?;
    screen.wait_byte()?;
    Ok(None)
}

/// AN-11: measures `path` on another thread, showing progress. `q` or Ctrl-C stops it
/// and gives `None`; other keys pressed meanwhile are kept in `queued`.
fn measure(
    screen: &Screen,
    path: &Path,
    queued: &mut Vec<Key>,
) -> Result<Option<Node>, Box<dyn Error>> {
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
            screen.draw(&[line])?;
            for key in read_keys(screen)? {
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

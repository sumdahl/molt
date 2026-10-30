//! The browser's state, keys and screen (specs/012-analyze.md AN-12 to AN-16). Pure: the
//! terminal and the filesystem are `terminal`'s, so tests drive this directly.

use std::path::PathBuf;

use super::{EntryKind, Node, share, size, suffix};
use crate::safe_delete::Candidate;
use crate::scan::human_size;

#[cfg(test)]
mod tests;

/// AN-13
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Down,
    Up,
    First,
    Last,
    PageDown,
    PageUp,
    Open,
    Back,
    Sort,
    Refresh,
    Help,
    /// AN-18
    Mark,
    /// AN-20
    Trash,
    Quit,
}

/// Decodes what the terminal sent: bytes, and the escape sequences of the special keys.
/// Anything else is ignored.
pub fn keys(bytes: &[u8]) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut rest = bytes;
    while let Some((&first, tail)) = rest.split_first() {
        rest = tail;
        let key = match first {
            b'j' => Key::Down,
            b'k' => Key::Up,
            b'g' => Key::First,
            b'G' => Key::Last,
            b'l' | b'\r' | b'\n' => Key::Open,
            b'h' | 0x7f | 0x08 => Key::Back,
            b's' => Key::Sort,
            b'r' => Key::Refresh,
            b'?' => Key::Help,
            b' ' => Key::Mark,
            b'd' => Key::Trash,
            // Raw mode turns Ctrl-C into a byte, so it never kills onyx mid-draw.
            b'q' | 0x03 => Key::Quit,
            0x1b => match escape(rest) {
                Some((key, len)) => {
                    rest = &rest[len..];
                    key
                }
                None => continue,
            },
            _ => continue,
        };
        keys.push(key);
    }
    keys
}

/// The key an escape sequence names, and how many bytes after ESC it took.
fn escape(rest: &[u8]) -> Option<(Key, usize)> {
    const SEQUENCES: [(&[u8], Key); 14] = [
        (b"[A", Key::Up),
        (b"OA", Key::Up),
        (b"[B", Key::Down),
        (b"OB", Key::Down),
        (b"[C", Key::Open),
        (b"OC", Key::Open),
        (b"[D", Key::Back),
        (b"OD", Key::Back),
        (b"[H", Key::First),
        (b"[1~", Key::First),
        (b"[F", Key::Last),
        (b"[4~", Key::Last),
        (b"[5~", Key::PageUp),
        (b"[6~", Key::PageDown),
    ];
    SEQUENCES
        .iter()
        .find(|(sequence, _)| rest.starts_with(sequence))
        .map(|(sequence, key)| (*key, sequence.len()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Size,
    Name,
}

/// What the terminal side must do after a key.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    Redraw,
    /// Read the files of this folder, then call `Browser::show`.
    Load(PathBuf),
    /// Measure this folder again, then call `Browser::replace`.
    Measure(PathBuf),
    /// AN-19: record this entry (`Candidate::scan`), then call `Browser::mark`.
    Mark(PathBuf),
    /// AN-20: preview and trash `Browser::picked`.
    Trash,
    Quit,
}

/// One folder on screen.
#[derive(Debug, Clone)]
struct View {
    /// Names from the starting folder down to this one.
    trail: Vec<String>,
    entries: Vec<Node>,
    /// The files read when it was opened (AN-17).
    files: Vec<Node>,
    selected: usize,
    top: usize,
}

#[derive(Debug)]
pub struct Browser {
    root: PathBuf,
    /// Every folder, in name order, with no files (AN-17).
    tree: Node,
    view: View,
    /// The folders above, to go back to (AN-13: never above the start).
    above: Vec<View>,
    order: Order,
    help: bool,
    /// Rows the entry list has, set by the terminal side.
    rows: usize,
    /// AN-18, AN-19: marked entries, as they were when marked, in marking order.
    marks: Vec<Marked>,
}

#[derive(Debug, Clone)]
struct Marked {
    candidate: Candidate,
    bytes: u64,
}

impl Browser {
    /// `tree` is `measure_folders` of `root`, and `files` the files directly in it.
    pub fn new(root: PathBuf, tree: Node, files: Vec<Node>) -> Self {
        let mut browser = Self {
            root,
            tree,
            view: View {
                trail: Vec::new(),
                entries: Vec::new(),
                files: Vec::new(),
                selected: 0,
                top: 0,
            },
            above: Vec::new(),
            order: Order::Size,
            help: false,
            rows: 10,
            marks: Vec::new(),
        };
        browser.show(files);
        browser
    }

    pub fn path(&self) -> PathBuf {
        self.path_of(&self.view.trail)
    }

    pub fn set_rows(&mut self, rows: usize) {
        self.rows = rows.max(1);
        self.scroll();
    }

    /// Shows the current folder with `files`, read just now.
    pub fn show(&mut self, files: Vec<Node>) {
        self.view.files = files;
        self.rebuild();
    }

    /// AN-13 `r`: puts a fresh `measure_folders` of the current folder in the tree, and
    /// carries the change in size up to the folders above.
    pub fn replace(&mut self, fresh: Node, files: Vec<Node>) {
        let trail = self.view.trail.clone();
        self.replace_at(&trail, fresh);
        self.show(files);
    }

    /// The folder `trail` names, measured again as `fresh`; the folders above it gain
    /// the change in size. The current view is rebuilt from the tree.
    pub fn replace_at(&mut self, trail: &[String], fresh: Node) {
        let mut node = &mut self.tree;
        let old = folder(node, trail).map_or(0, |found| found.bytes);
        for name in trail {
            // Each folder above loses the old size and gains the new.
            node.bytes = node.bytes.saturating_sub(old) + fresh.bytes;
            let Some(next) = child_mut(node, name) else {
                return;
            };
            node = next;
        }
        let name = std::mem::take(&mut node.name);
        *node = fresh;
        node.name = name;
        self.rebuild();
    }

    /// The path a trail names.
    pub fn path_of(&self, trail: &[String]) -> PathBuf {
        trail
            .iter()
            .fold(self.root.clone(), |path, name| path.join(name))
    }

    pub fn press(&mut self, key: Key) -> Effect {
        if self.help {
            self.help = false;
            return if key == Key::Quit {
                Effect::Quit
            } else {
                Effect::Redraw
            };
        }
        let last = self.view.entries.len().saturating_sub(1);
        let selected = &mut self.view.selected;
        match key {
            Key::Down => *selected = (*selected + 1).min(last),
            Key::Up => *selected = selected.saturating_sub(1),
            Key::First => *selected = 0,
            Key::Last => *selected = last,
            Key::PageDown => *selected = (*selected + self.rows).min(last),
            Key::PageUp => *selected = selected.saturating_sub(self.rows),
            Key::Open => return self.open(),
            Key::Back => self.back(),
            Key::Sort => {
                self.order = match self.order {
                    Order::Size => Order::Name,
                    Order::Name => Order::Size,
                };
                self.rebuild();
            }
            Key::Refresh => return Effect::Measure(self.path()),
            Key::Help => self.help = true,
            Key::Mark => return self.toggle_mark(),
            Key::Trash if self.marks.is_empty() && self.view.entries.is_empty() => {}
            Key::Trash => return Effect::Trash,
            Key::Quit => return Effect::Quit,
        }
        self.scroll();
        Effect::Redraw
    }

    /// AN-18: unmarks the selected entry if it's marked; otherwise asks for it to be
    /// recorded. Either way the selection moves down.
    fn toggle_mark(&mut self) -> Effect {
        let Some(path) = self.selected_path() else {
            return Effect::Redraw;
        };
        let before = self.marks.len();
        self.marks.retain(|mark| mark.candidate.path() != path);
        if self.marks.len() < before {
            self.press(Key::Down);
            return Effect::Redraw;
        }
        Effect::Mark(path)
    }

    /// AN-19: keeps `candidate`, recorded just now, as a mark, and moves down.
    pub fn mark(&mut self, candidate: Candidate) {
        let bytes = self
            .view
            .entries
            .iter()
            .find(|entry| self.path().join(&entry.name) == candidate.path())
            .map_or(0, |entry| entry.bytes);
        self.marks.push(Marked { candidate, bytes });
        self.press(Key::Down);
    }

    /// AN-20: what `d` trashes: the marks, or else the selected entry.
    pub fn picked(&self) -> Vec<Candidate> {
        if self.marks.is_empty() {
            return self
                .selected_path()
                .and_then(|path| Candidate::scan(path).ok())
                .into_iter()
                .collect();
        }
        self.marks
            .iter()
            .map(|mark| mark.candidate.clone())
            .collect()
    }

    /// AN-22: after the trash step, the marks are gone.
    pub fn clear_marks(&mut self) {
        self.marks.clear();
    }

    /// AN-22: the deepest folder holding every picked path, as names under the start.
    pub fn common_trail(&self, paths: &[PathBuf]) -> Vec<String> {
        let mut trail: Option<Vec<String>> = None;
        for path in paths {
            let Ok(below) = path.strip_prefix(&self.root) else {
                return Vec::new();
            };
            let mut names: Vec<String> = below
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            // The folder holding it, not the item itself.
            names.pop();
            trail = Some(match trail {
                None => names,
                Some(common) => common
                    .into_iter()
                    .zip(names)
                    .take_while(|(a, b)| a == b)
                    .map(|(a, _)| a)
                    .collect(),
            });
        }
        trail.unwrap_or_default()
    }

    fn selected_path(&self) -> Option<PathBuf> {
        let entry = self.view.entries.get(self.view.selected)?;
        Some(self.path().join(&entry.name))
    }

    fn is_marked(&self, entry: &Node) -> bool {
        let path = self.path().join(&entry.name);
        self.marks.iter().any(|mark| mark.candidate.path() == path)
    }

    fn open(&mut self) -> Effect {
        let Some(entry) = self.view.entries.get(self.view.selected) else {
            return Effect::Redraw;
        };
        if entry.kind != EntryKind::Dir {
            return Effect::Redraw;
        }
        let mut trail = self.view.trail.clone();
        trail.push(entry.name.clone());
        let below = View {
            trail,
            entries: Vec::new(),
            files: Vec::new(),
            selected: 0,
            top: 0,
        };
        self.above.push(std::mem::replace(&mut self.view, below));
        Effect::Load(self.path())
    }

    fn back(&mut self) {
        let Some(view) = self.above.pop() else {
            return;
        };
        let from = self.view.trail.last().cloned();
        self.view = view;
        // Sizes may have changed below (`r`), so rebuild, keeping the folder we came from.
        self.rebuild();
        if let Some(index) = from.and_then(|name| self.index_of(&name)) {
            self.view.selected = index;
        }
        self.scroll();
    }

    /// The entries: subfolders from the tree, then the files, in the chosen order. The
    /// selection stays on the same name.
    fn rebuild(&mut self) {
        let keep = self
            .view
            .entries
            .get(self.view.selected)
            .map(|entry| entry.name.clone());
        let folders = folder(&self.tree, &self.view.trail)
            .map(|node| node.children.iter().map(Node::summary).collect::<Vec<_>>())
            .unwrap_or_default();
        let mut entries: Vec<Node> = folders.into_iter().chain(self.view.files.clone()).collect();
        match self.order {
            Order::Size => {
                entries.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
            }
            Order::Name => entries.sort_by(|a, b| a.name.cmp(&b.name)),
        }
        self.view.entries = entries;
        self.view.selected = keep
            .and_then(|name| self.index_of(&name))
            .unwrap_or(0)
            .min(self.view.entries.len().saturating_sub(1));
        self.scroll();
    }

    fn index_of(&self, name: &str) -> Option<usize> {
        self.view
            .entries
            .iter()
            .position(|entry| entry.name == name)
    }

    /// Keeps the selection on screen.
    fn scroll(&mut self) {
        let view = &mut self.view;
        if view.selected < view.top {
            view.top = view.selected;
        } else if view.selected >= view.top + self.rows {
            view.top = view.selected + 1 - self.rows;
        }
    }

    /// AN-12, AN-16: the screen, `height` lines of at most `width` characters.
    pub fn lines(&self, width: usize, height: usize) -> Vec<String> {
        let current = folder(&self.tree, &self.view.trail).unwrap_or(&self.tree);
        let mut lines = vec![fit(
            &format!(
                "{}  {}",
                size(current).trim_start(),
                self.path().to_string_lossy()
            ),
            width,
        )];
        if self.help {
            lines.extend(HELP.iter().map(|line| fit(line, width)));
        } else {
            let rows = height.saturating_sub(2);
            for (index, entry) in self
                .view
                .entries
                .iter()
                .enumerate()
                .skip(self.view.top)
                .take(rows)
            {
                let mark = if self.is_marked(entry) { '*' } else { ' ' };
                let line = fit(
                    &format!(
                        "{mark}{}",
                        row(entry, current.bytes, width.saturating_sub(1))
                    ),
                    width,
                );
                lines.push(if index == self.view.selected {
                    format!("\x1b[7m{line}\x1b[0m")
                } else {
                    line
                });
            }
            if self.view.entries.is_empty() {
                lines.push("  (empty)".to_owned());
            }
        }
        lines.truncate(height.saturating_sub(1));
        while lines.len() < height.saturating_sub(1) {
            lines.push(String::new());
        }
        let order = match self.order {
            Order::Size => "by size",
            Order::Name => "by name",
        };
        let marked = if self.marks.is_empty() {
            String::new()
        } else {
            let bytes: u64 = self.marks.iter().map(|mark| mark.bytes).sum();
            format!("{} marked, {}  ·  ", self.marks.len(), human_size(bytes))
        };
        lines.push(fit(
            &format!(
                "{marked}{order}  ·  j/k move  l open  h up  space mark  d trash  ? help  q quit"
            ),
            width,
        ));
        lines
    }
}

const HELP: [&str; 11] = [
    "",
    "  j k ↓ ↑        move",
    "  g G Home End   first, last",
    "  PgUp PgDn      a screen up, down",
    "  l → Enter      open the folder",
    "  h ← Backspace  go up",
    "  s              sort by size or name",
    "  r              measure this folder again",
    "  Space          mark or unmark, then move down",
    "  d              move the marked items (or this one) to the trash, after a preview",
    "  q Ctrl-C       quit (any other key closes this help)",
];

/// Size, share of the folder, a bar, and the name (AN-12), cut to `width`.
fn row(entry: &Node, total: u64, width: usize) -> String {
    const BAR: usize = 10;
    let filled = if total == 0 {
        0
    } else {
        // Rounds to the nearest bar step; `bytes <= total` keeps it within BAR.
        usize::try_from(
            (u128::from(entry.bytes) * BAR as u128 + u128::from(total) / 2) / u128::from(total),
        )
        .unwrap_or(BAR)
        .min(BAR)
    };
    let mut prefix = format!("{}  {:>6}  ", size(entry), share(entry.bytes, total));
    // A narrow terminal keeps the name and drops the bar.
    if width >= prefix.chars().count() + BAR + 4 + 20 {
        prefix.push_str(&format!(
            "[{}{}]  ",
            "#".repeat(filled),
            " ".repeat(BAR - filled)
        ));
    }
    let name = format!("{}{}", entry.name, suffix(entry.kind));
    let room = width.saturating_sub(prefix.chars().count());
    fit(&format!("{prefix}{}", shorten(&name, room)), width)
}

/// AN-16: cuts `text` to `width` characters.
fn fit(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}

/// AN-16: shortens `name` in the middle with `…` to fit `room` characters.
// ponytail: counts chars, not terminal columns; wide (CJK) names may overflow a little.
fn shorten(name: &str, room: usize) -> String {
    let len = name.chars().count();
    if len <= room {
        return name.to_owned();
    }
    if room < 2 {
        return "…".chars().take(room).collect();
    }
    let head = (room - 1) / 2;
    let tail = room - 1 - head;
    let start: String = name.chars().take(head).collect();
    let end: String = name.chars().skip(len - tail).collect();
    format!("{start}…{end}")
}

/// The folder `trail` names under `tree`. Children are in name order (`walk`).
fn folder<'a>(tree: &'a Node, trail: &[String]) -> Option<&'a Node> {
    trail.iter().try_fold(tree, |node, name| {
        node.children
            .binary_search_by(|child| child.name.as_str().cmp(name))
            .ok()
            .map(|index| &node.children[index])
    })
}

fn child_mut<'a>(node: &'a mut Node, name: &str) -> Option<&'a mut Node> {
    let index = node
        .children
        .binary_search_by(|child| child.name.as_str().cmp(name))
        .ok()?;
    node.children.get_mut(index)
}

impl Node {
    /// The entry for a folder, without what's inside it.
    fn summary(&self) -> Self {
        let mut entry = Self::new(self.name.clone(), self.kind, self.bytes);
        entry.partial = self.partial;
        entry
    }
}

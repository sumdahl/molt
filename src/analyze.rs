//! `molt analyze` (specs/012-analyze.md): where the space in a folder goes. Read-only.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::{Scope, ScopedJoinHandle};

use serde::Serialize;

use crate::scan::human_size;

mod browse;
mod terminal;
#[cfg(test)]
mod tests;

pub use terminal::browse;

/// AN-07: entries shown per level without `--all`.
const SHOWN: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    /// AN-02: another filesystem, listed but never entered.
    Mount,
    Other,
}

/// A measured entry. Folders deeper than the requested depth have no `children`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    pub name: String,
    pub kind: EntryKind,
    pub bytes: u64,
    /// AN-03: something inside couldn't be measured, so this is a lower bound.
    pub partial: bool,
    pub children: Vec<Node>,
    /// Files with more than one link, counted only once the whole tree is known (AN-01).
    #[serde(skip)]
    links: Vec<Link>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Link {
    name: String,
    inode: (u64, u64),
    bytes: u64,
}

#[derive(Debug)]
pub enum AnalyzeError {
    /// AN-05
    NotADirectory(PathBuf),
    Unreadable(PathBuf, io::Error),
}

impl fmt::Display for AnalyzeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotADirectory(path) => write!(f, "{} is not a directory", path.display()),
            Self::Unreadable(path, e) => write!(f, "can't read {}: {e}", path.display()),
        }
    }
}

impl std::error::Error for AnalyzeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotADirectory(_) => None,
            Self::Unreadable(_, e) => Some(e),
        }
    }
}

/// Measures `root` with its children down to `depth` levels, largest first.
pub fn measure(root: &Path, depth: usize) -> Result<Node, AnalyzeError> {
    let mut tree = walk(root, depth, &Progress::default())?;
    trim(&mut tree, depth);
    Ok(tree)
}

/// AN-17: every folder under `root` with its total, in name order, but no files.
pub(crate) fn measure_folders(root: &Path, progress: &Progress) -> Result<Node, AnalyzeError> {
    walk(root, 0, progress)
}

/// AN-11: what the walk has counted so far, and a flag that stops it.
#[derive(Debug, Default)]
pub(crate) struct Progress {
    pub folders: AtomicU64,
    pub bytes: AtomicU64,
    pub stop: AtomicBool,
}

/// The settled walk of `root`, its children in name order, files kept to `depth`.
fn walk(root: &Path, depth: usize, progress: &Progress) -> Result<Node, AnalyzeError> {
    let meta =
        fs::symlink_metadata(root).map_err(|e| AnalyzeError::Unreadable(root.to_owned(), e))?;
    // AN-05: a symlink is refused too; analyze never follows one.
    if !meta.is_dir() {
        return Err(AnalyzeError::NotADirectory(root.to_owned()));
    }
    let spare = std::thread::available_parallelism().map_or(0, |n| n.get() - 1);
    let walker = Walker {
        dev: meta.dev(),
        spare: AtomicUsize::new(spare),
        progress,
    };
    let name = root.to_string_lossy().into_owned();
    let mut tree = std::thread::scope(|scope| {
        walker.dir(scope, root.to_owned(), name, meta.blocks() * 512, depth)
    });
    settle(&mut tree, &mut HashSet::new());
    Ok(tree)
}

/// AN-02, AN-04: one walk over one filesystem; `spare` counts the threads still free.
struct Walker<'p> {
    dev: u64,
    spare: AtomicUsize,
    progress: &'p Progress,
}

enum Pending<'scope> {
    Done(Node),
    Running(ScopedJoinHandle<'scope, Node>),
}

impl Walker<'_> {
    /// Walks the folder at `path`. Every subfolder is kept (settle needs them all); files
    /// only while `depth` is above 0. Hard-linked files wait in `links` for `settle`.
    fn dir<'scope, 'env>(
        &'env self,
        scope: &'scope Scope<'scope, 'env>,
        path: PathBuf,
        name: String,
        own: u64,
        depth: usize,
    ) -> Node {
        let mut node = Node::new(name, EntryKind::Dir, own);
        self.progress.folders.fetch_add(1, Ordering::Relaxed);
        self.progress.bytes.fetch_add(own, Ordering::Relaxed);
        // AN-11: a stopped walk is a partial one.
        if self.progress.stop.load(Ordering::Relaxed) {
            node.partial = true;
            return node;
        }
        let Ok(entries) = fs::read_dir(&path) else {
            node.partial = true;
            return node;
        };
        let mut pending = Vec::new();
        for entry in entries {
            let Ok(entry) = entry else {
                node.partial = true;
                continue;
            };
            let child_path = entry.path();
            let child_name = entry.file_name().to_string_lossy().into_owned();
            let Ok(meta) = fs::symlink_metadata(&child_path) else {
                node.partial = true;
                continue;
            };
            let bytes = meta.blocks() * 512;
            let kind = kind_of(&meta, self.dev);
            match kind {
                EntryKind::Dir => {
                    let below = depth.saturating_sub(1);
                    pending.push(self.subdir(scope, child_path, child_name, bytes, below));
                    continue;
                }
                EntryKind::Mount => {
                    if depth > 0 {
                        node.children.push(Node::new(child_name, kind, 0));
                    }
                    continue;
                }
                EntryKind::File if meta.nlink() > 1 => {
                    node.links.push(Link {
                        name: child_name.clone(),
                        inode: (meta.dev(), meta.ino()),
                        bytes,
                    });
                    if depth > 0 {
                        node.children.push(Node::new(child_name, kind, 0));
                    }
                    continue;
                }
                EntryKind::File | EntryKind::Symlink | EntryKind::Other => {}
            }
            node.bytes += bytes;
            self.progress.bytes.fetch_add(bytes, Ordering::Relaxed);
            if depth > 0 {
                node.children.push(Node::new(child_name, kind, bytes));
            }
        }
        for subdir in pending {
            node.children.push(match subdir {
                Pending::Done(child) => child,
                Pending::Running(handle) => handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            });
        }
        // AN-04: name order, whatever order the threads finished in.
        node.children.sort_by(|a, b| a.name.cmp(&b.name));
        node.links.sort_by(|a, b| a.name.cmp(&b.name));
        node
    }

    /// Walks a subfolder on a spare thread if there is one, or else on this one.
    fn subdir<'scope, 'env>(
        &'env self,
        scope: &'scope Scope<'scope, 'env>,
        path: PathBuf,
        name: String,
        own: u64,
        depth: usize,
    ) -> Pending<'scope> {
        if !self.claim_thread() {
            return Pending::Done(self.dir(scope, path, name, own, depth));
        }
        Pending::Running(scope.spawn(move || {
            let node = self.dir(scope, path, name, own, depth);
            self.spare.fetch_add(1, Ordering::AcqRel);
            node
        }))
    }
}

impl Walker<'_> {
    /// Takes one spare thread, if any is left.
    fn claim_thread(&self) -> bool {
        let mut spare = self.spare.load(Ordering::Acquire);
        while spare > 0 {
            match self.spare.compare_exchange_weak(
                spare,
                spare - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(now) => spare = now,
            }
        }
        false
    }
}

/// AN-17: the entries of `dir` other than its subfolders on this filesystem, read now.
/// Each file counts at its own size. What can't be read is left out; the folder's total
/// already says it's partial.
pub(crate) fn files(dir: &Path, dev: u64) -> Vec<Node> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let meta = fs::symlink_metadata(entry.path()).ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            match kind_of(&meta, dev) {
                EntryKind::Dir => None,
                EntryKind::Mount => Some(Node::new(name, EntryKind::Mount, 0)),
                kind => Some(Node::new(name, kind, meta.blocks() * 512)),
            }
        })
        .collect()
}

fn kind_of(meta: &fs::Metadata, dev: u64) -> EntryKind {
    let kind = meta.file_type();
    if kind.is_symlink() {
        EntryKind::Symlink
    } else if kind.is_dir() && meta.dev() != dev {
        EntryKind::Mount
    } else if kind.is_dir() {
        EntryKind::Dir
    } else if kind.is_file() {
        EntryKind::File
    } else {
        // Fifos, sockets and devices.
        EntryKind::Other
    }
}

/// AN-01, AN-03: adds folder totals in name order, counting each hard-linked file for the
/// first path that reaches it, and marks the folders above anything partial.
fn settle(node: &mut Node, seen: &mut HashSet<(u64, u64)>) {
    let Node {
        bytes,
        partial,
        children,
        links,
        ..
    } = node;
    let mut links = std::mem::take(links).into_iter().peekable();
    for child in children {
        // Links not shown (below the depth) that sort before this child come first.
        while let Some(link) = links.next_if(|link| link.name < child.name) {
            count_link(bytes, None, &link, seen);
        }
        if let Some(link) = links.next_if(|link| link.name == child.name) {
            count_link(bytes, Some(child), &link, seen);
        } else if child.kind == EntryKind::Dir {
            settle(child, seen);
            *bytes += child.bytes;
            *partial |= child.partial;
        }
    }
    for link in links {
        count_link(bytes, None, &link, seen);
    }
}

fn count_link(
    total: &mut u64,
    shown: Option<&mut Node>,
    link: &Link,
    seen: &mut HashSet<(u64, u64)>,
) {
    if seen.insert(link.inode) {
        *total += link.bytes;
        if let Some(node) = shown {
            node.bytes = link.bytes;
        }
    }
}

/// Drops what lies below `depth`, and puts each level largest first, then by name (AN-06).
fn trim(node: &mut Node, depth: usize) {
    if depth == 0 {
        node.children.clear();
        return;
    }
    for child in &mut node.children {
        trim(child, depth - 1);
    }
    node.children
        .sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
}

impl Node {
    fn new(name: String, kind: EntryKind, bytes: u64) -> Self {
        Self {
            name,
            kind,
            bytes,
            partial: false,
            children: Vec::new(),
            links: Vec::new(),
        }
    }
}

/// AN-06, AN-07: the text listing of a measured tree.
pub struct Listing<'a> {
    tree: &'a Node,
    depth: usize,
    all: bool,
}

impl<'a> Listing<'a> {
    pub fn new(tree: &'a Node, depth: usize, all: bool) -> Self {
        Self { tree, depth, all }
    }

    fn level(&self, f: &mut fmt::Formatter<'_>, node: &Node, indent: usize) -> fmt::Result {
        let shown = if self.all { node.children.len() } else { SHOWN };
        for child in node.children.iter().take(shown) {
            writeln!(
                f,
                "{}  {:>5}  {:indent$}{}{}",
                size(child),
                share(child.bytes, self.tree.bytes),
                "",
                child.name,
                suffix(child.kind),
                indent = indent * 2
            )?;
            if indent + 1 < self.depth {
                self.level(f, child, indent + 1)?;
            }
        }
        let rest = &node.children[shown.min(node.children.len())..];
        if !rest.is_empty() {
            let bytes: u64 = rest.iter().map(|child| child.bytes).sum();
            writeln!(
                f,
                "{:19}{:indent$}… {} more, {}",
                "",
                "",
                rest.len(),
                human_size(bytes),
                indent = indent * 2
            )?;
        }
        Ok(())
    }
}

impl fmt::Display for Listing<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}         {}", size(self.tree), self.tree.name)?;
        self.level(f, self.tree, 0)
    }
}

/// AN-03: `+` marks a lower bound.
fn size(node: &Node) -> String {
    let plus = if node.partial { "+" } else { " " };
    format!("{:>10}{plus}", human_size(node.bytes))
}

fn share(bytes: u64, total: u64) -> String {
    if total == 0 {
        return "0.0%".to_owned();
    }
    // Display precision only.
    #[allow(clippy::cast_precision_loss)]
    let percent = bytes as f64 * 100.0 / total as f64;
    format!("{percent:.1}%")
}

fn suffix(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Dir => "/",
        EntryKind::Mount => "/  (mount)",
        EntryKind::Symlink => "@",
        EntryKind::File | EntryKind::Other => "",
    }
}

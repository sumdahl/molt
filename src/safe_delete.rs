//! Safe delete (specs/001-safe-delete.md): the only module allowed to remove files (P-01).

// P-01: this is the one module allowed to call the removal functions clippy bans elsewhere.
#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::fmt;
use std::iter;
use std::os::fd::OwnedFd;
use std::path::{Component, Path, PathBuf};

pub use rustix::fs::Uid;
use rustix::fs::{
    AtFlags, CWD, Dir, FileType, Mode, OFlags, Stat, fstat, openat, statat, unlinkat,
};
pub use rustix::io::Errno;
use rustix::path::Arg;

#[cfg(test)]
mod tests;

/// What a scan saw at a path, rechecked right before removal (SD-08, SD-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    file_type: FileType,
    mtime: (i64, u32),
}

impl From<&Stat> for Identity {
    // Stat field widths differ between architectures.
    #[allow(clippy::unnecessary_cast)]
    fn from(stat: &Stat) -> Self {
        Self {
            dev: stat.st_dev as u64,
            ino: stat.st_ino as u64,
            file_type: FileType::from_raw_mode(stat.st_mode as _),
            mtime: (stat.st_mtime as i64, stat.st_mtime_nsec as u32),
        }
    }
}

impl Identity {
    /// SD-08, SD-10: the object at the path must be the one the scan saw, unmodified.
    fn verify(&self, stat: &Stat) -> Result<(), Stop> {
        let now = Self::from(stat);
        if (now.dev, now.ino, now.file_type) != (self.dev, self.ino, self.file_type) {
            Err(Stop::Skip(Reason::ChangedSinceScan))
        } else if now.mtime > self.mtime {
            Err(Stop::Skip(Reason::ModifiedSinceScan))
        } else {
            Ok(())
        }
    }
}

/// A path found by a scan, with the identity it had at scan time.
#[derive(Debug, Clone)]
pub struct Candidate {
    path: PathBuf,
    identity: Identity,
}

impl Candidate {
    /// Records `path` as it is now. A final symlink is recorded as the link itself.
    pub fn scan(path: impl Into<PathBuf>) -> Result<Self, Errno> {
        let path = path.into();
        let identity = Identity::from(&rustix::fs::lstat(&path)?);
        Ok(Self { path, identity })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Whether a plan changes the filesystem (SD-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Execution {
    DryRun,
    Live,
}

/// Who created the plan. Only an explicit system step may run as root (SD-16, P-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Privilege {
    User,
    System,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub candidates: Vec<Candidate>,
    pub execution: Execution,
    pub privilege: Privilege,
}

/// The user environment the guard protects. Passed in, never read from the process,
/// so tests can point it at a temporary home.
#[derive(Debug, Clone)]
pub struct Context {
    pub home: PathBuf,
    /// XDG base directories (config, cache, data, state…).
    pub xdg_dirs: Vec<PathBuf>,
    /// Paths the user's config says to never touch (P-03, SD-03).
    pub whitelist: Vec<PathBuf>,
    pub euid: Uid,
}

/// Why a candidate was left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Protected,
    Whitelisted,
    InvalidPath,
    MountPoint,
    ChangedSinceScan,
    ModifiedSinceScan,
}

/// What happened to one candidate. `bytes` are on-disk bytes freed (SD-13).
/// `partial` means something inside was kept, such as a mount (SD-07).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Removed { bytes: u64, partial: bool },
    WouldRemove { bytes: u64, partial: bool },
    Skipped(Reason),
    Failed { errno: Errno, bytes: u64 },
}

#[derive(Debug, Clone)]
pub struct Report {
    pub results: Vec<(PathBuf, Outcome)>,
}

impl Report {
    /// P-13: 3 when any candidate failed, otherwise 0.
    pub fn exit_code(&self) -> i32 {
        let failed = self
            .results
            .iter()
            .any(|(_, outcome)| matches!(outcome, Outcome::Failed { .. }));
        if failed { 3 } else { 0 }
    }
}

/// A plan refused as a whole, before anything was touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanRefused {
    /// SD-16: running as root without an explicit system step.
    Root,
}

impl fmt::Display for PlanRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root => f.write_str("refusing to run as root without an explicit system step"),
        }
    }
}

impl std::error::Error for PlanRefused {}

/// Carries out `plan`, one candidate at a time. A failure on one candidate never stops
/// the others (SD-14).
pub fn execute(plan: &Plan, ctx: &Context) -> Result<Report, PlanRefused> {
    #[cfg(test)]
    tests::assert_sandboxed(plan);
    if ctx.euid.is_root() && plan.privilege != Privilege::System {
        return Err(PlanRefused::Root);
    }
    let guard = Guard::new(ctx);
    let mut remover = Remover {
        execution: plan.execution,
        links: HashMap::new(),
    };
    let results = plan
        .candidates
        .iter()
        .map(|candidate| (candidate.path.clone(), remover.run(candidate, &guard)))
        .collect();
    Ok(Report { results })
}

/// Directories whose `$HOME`-relative contents are secret (SD-02).
const SECRETS: [&str; 4] = [".ssh", ".gnupg", ".password-store", ".local/share/keyrings"];

/// Files in a Firefox profile that hold saved passwords (SD-02).
const FIREFOX_SECRETS: [&str; 2] = ["key4.db", "logins.json"];

/// Decides which paths may never be removed (SD-01 to SD-03). Compares canonical paths.
struct Guard {
    /// Protected themselves and every directory above them, but not their contents.
    roots: Vec<PathBuf>,
    /// Protected themselves, their contents, and every directory above them.
    secrets: Vec<PathBuf>,
    firefox_profiles: PathBuf,
    whitelist: Vec<PathBuf>,
}

impl Guard {
    fn new(ctx: &Context) -> Self {
        let home = canonical(&ctx.home);
        Self {
            roots: iter::once(home.clone())
                .chain(ctx.xdg_dirs.iter().map(|dir| canonical(dir)))
                .collect(),
            secrets: SECRETS.iter().map(|secret| home.join(secret)).collect(),
            firefox_profiles: home.join(".mozilla/firefox"),
            whitelist: ctx.whitelist.iter().map(|path| canonical(path)).collect(),
        }
    }

    fn check(&self, path: &Path) -> Result<(), Reason> {
        let at_or_above = |guarded: &PathBuf| guarded.starts_with(path);
        let related = |guarded: &PathBuf| guarded.starts_with(path) || path.starts_with(guarded);
        // SD-01: `/` and the top-level directories have at most two components.
        let protected = path.components().count() <= 2
            || self.roots.iter().any(at_or_above)
            || self.secrets.iter().any(related)
            || self.is_firefox_secret(path);
        if protected {
            Err(Reason::Protected)
        } else if self.whitelist.iter().any(related) {
            Err(Reason::Whitelisted)
        } else {
            Ok(())
        }
    }

    /// A profile directory, anything above one, or a password file inside one.
    fn is_firefox_secret(&self, path: &Path) -> bool {
        match path.strip_prefix(&self.firefox_profiles) {
            Ok(rel) => {
                rel.components().count() <= 1
                    || path
                        .file_name()
                        .is_some_and(|name| FIREFOX_SECRETS.iter().any(|secret| name == *secret))
            }
            Err(_) => self.firefox_profiles.starts_with(path),
        }
    }
}

/// A path that doesn't exist yet can't be resolved, but still names what to protect.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// SD-04: absolute, valid UTF-8, and free of `..` and leading `.` components.
fn is_valid(path: &Path) -> bool {
    path.to_str().is_some()
        && path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}

/// Why work on one candidate ended early.
enum Stop {
    Skip(Reason),
    Fail(Errno),
}

impl From<Errno> for Stop {
    fn from(errno: Errno) -> Self {
        Self::Fail(errno)
    }
}

/// Running totals for one candidate.
#[derive(Default)]
struct Tally {
    bytes: u64,
    partial: bool,
}

/// Opens path components for `*at` calls only, never through a symlink.
const WALK: OFlags = OFlags::PATH
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// Opens a directory to list it, never through a symlink.
const LIST: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

struct Remover {
    execution: Execution,
    /// SD-13: hard-linked inodes seen so far, as (link count at first sight, links seen).
    links: HashMap<(u64, u64), (u64, u64)>,
}

impl Remover {
    fn run(&mut self, candidate: &Candidate, guard: &Guard) -> Outcome {
        let mut tally = Tally::default();
        match self.remove_candidate(candidate, guard, &mut tally) {
            Ok(()) => {
                let Tally { bytes, partial } = tally;
                match self.execution {
                    Execution::DryRun => Outcome::WouldRemove { bytes, partial },
                    Execution::Live => Outcome::Removed { bytes, partial },
                }
            }
            Err(Stop::Skip(reason)) => Outcome::Skipped(reason),
            Err(Stop::Fail(errno)) => Outcome::Failed {
                errno,
                bytes: tally.bytes,
            },
        }
    }

    fn remove_candidate(
        &mut self,
        candidate: &Candidate,
        guard: &Guard,
        tally: &mut Tally,
    ) -> Result<(), Stop> {
        let path = &candidate.path;
        if !is_valid(path) {
            return Err(Stop::Skip(Reason::InvalidPath));
        }
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(Stop::Skip(Reason::Protected));
        };
        // Resolve symlinks above the candidate so the guard judges where it really is.
        let parent =
            std::fs::canonicalize(parent).map_err(|_| Stop::Skip(Reason::ChangedSinceScan))?;
        guard.check(&parent.join(name)).map_err(Stop::Skip)?;

        let parent_fd = open_dir_nofollow(&parent)?;
        let stat = statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|errno| {
            if errno == Errno::NOENT {
                Stop::Skip(Reason::ChangedSinceScan)
            } else {
                Stop::Fail(errno)
            }
        })?;
        let dir = match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => Some(openat(&parent_fd, name, LIST, Mode::empty())?),
            _ => None,
        };
        // SD-08: verify the directory actually opened, not just the name looked up before.
        let stat = match &dir {
            Some(dir) => fstat(dir)?,
            None => stat,
        };
        candidate.identity.verify(&stat)?;
        if Identity::from(&stat).dev != Identity::from(&fstat(&parent_fd)?).dev {
            return Err(Stop::Skip(Reason::MountPoint));
        }

        match dir {
            Some(dir) => {
                if !self.remove_dir(&parent_fd, name, &dir, &stat, tally)? {
                    tally.partial = true;
                }
            }
            None => self.unlink(&parent_fd, name, &stat, AtFlags::empty(), tally)?,
        }
        Ok(())
    }

    /// Empties `dir`, then removes it from `parent` unless something inside had to stay.
    /// Returns whether it was removed.
    fn remove_dir(
        &mut self,
        parent: &OwnedFd,
        name: impl Arg,
        dir: &OwnedFd,
        stat: &Stat,
        tally: &mut Tally,
    ) -> Result<bool, Errno> {
        let emptied = self.remove_contents(dir, Identity::from(stat).dev, tally)?;
        if emptied {
            self.unlink(parent, name, stat, AtFlags::REMOVEDIR, tally)?;
        }
        Ok(emptied)
    }

    /// SD-09: each level is opened relative to its parent's handle with `O_NOFOLLOW`, and
    /// entries are removed with `unlinkat` on that handle, so a component swapped for a
    /// symlink mid-removal can't redirect anything outside the candidate.
    /// SD-07: entries on another filesystem (`dev`) are kept. Returns whether `dir` is now empty.
    // ponytail: one open fd per level of recursion; trees deeper than the fd limit fail with EMFILE.
    fn remove_contents(
        &mut self,
        dir: &OwnedFd,
        dev: u64,
        tally: &mut Tally,
    ) -> Result<bool, Errno> {
        let mut emptied = true;
        for name in entry_names(dir)? {
            let stat = statat(dir, &name, AtFlags::SYMLINK_NOFOLLOW)?;
            let identity = Identity::from(&stat);
            if identity.dev != dev {
                emptied = false;
            } else if identity.file_type != FileType::Directory {
                self.unlink(dir, &name, &stat, AtFlags::empty(), tally)?;
            } else {
                #[cfg(test)]
                tests::before_descend(&name);
                let sub = openat(dir, &name, LIST, Mode::empty())?;
                let sub_stat = fstat(&sub)?;
                emptied &= Identity::from(&sub_stat).dev == dev
                    && self.remove_dir(dir, &name, &sub, &sub_stat, tally)?;
            }
        }
        Ok(emptied)
    }

    /// Removes one entry (SD-12: only in a live run) and counts what it frees.
    fn unlink(
        &mut self,
        dir: &OwnedFd,
        name: impl Arg,
        stat: &Stat,
        flags: AtFlags,
        tally: &mut Tally,
    ) -> Result<(), Errno> {
        if self.execution == Execution::Live {
            unlinkat(dir, name, flags)?;
        }
        tally.bytes += self.freed(stat);
        Ok(())
    }

    /// SD-13: an inode's blocks count once, when the last of its links in the plan goes.
    fn freed(&mut self, stat: &Stat) -> u64 {
        let (bytes, nlink) = blocks_and_links(stat);
        let identity = Identity::from(stat);
        // A directory's link count comes from its subdirectories, not from hard links.
        if identity.file_type == FileType::Directory || nlink <= 1 {
            return bytes;
        }
        let (total, seen) = self
            .links
            .entry((identity.dev, identity.ino))
            .or_insert((nlink, 0));
        *seen += 1;
        if seen == total { bytes } else { 0 }
    }
}

/// On-disk bytes (P-12) and link count.
// Stat field widths differ between architectures.
#[allow(clippy::unnecessary_cast)]
fn blocks_and_links(stat: &Stat) -> (u64, u64) {
    (stat.st_blocks as u64 * 512, stat.st_nlink as u64)
}

/// Opens every component of a canonical absolute path without following symlinks, so the
/// directory the guard checked is the one used.
fn open_dir_nofollow(path: &Path) -> Result<OwnedFd, Errno> {
    let mut fd = openat(CWD, "/", WALK, Mode::empty())?;
    for component in path.components() {
        if let Component::Normal(name) = component {
            fd = openat(&fd, name, WALK, Mode::empty())?;
        }
    }
    Ok(fd)
}

/// Names in `dir`, read up front so removing entries can't disturb the listing.
fn entry_names(dir: &OwnedFd) -> Result<Vec<CString>, Errno> {
    let mut names = Vec::new();
    for entry in Dir::read_from(dir)? {
        let entry = entry?;
        let name: &CStr = entry.file_name();
        if name != c"." && name != c".." {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

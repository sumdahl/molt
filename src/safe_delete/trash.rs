//! SD-11: moving items to the trash, following the FreeDesktop Trash spec, so a normal
//! file manager can restore them.

use std::fmt::Write as _;
use std::fs::File;
use std::io::Write as _;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use rustix::fs::{
    AtFlags, Mode as FileMode, OFlags, RenameFlags, Uid, fstat, mkdirat, openat, renameat_with,
    stat, unlinkat,
};
use rustix::io::Errno;

use super::{Execution, Identity, WALK, canonical, open_dir_nofollow};

/// Gives up on finding a free name after this many suffixes.
const MAX_NAMES: u32 = 10_000;

/// Creates a `.trashinfo` only if the name is free, never through a symlink.
const NEW_INFO: OFlags = OFlags::CREATE
    .union(OFlags::EXCL)
    .union(OFlags::WRONLY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// Every trash the user may have: `$XDG_DATA_HOME/Trash` for the home filesystem,
/// `$topdir/.Trash-$uid` on any other.
pub(super) struct Bins {
    home: PathBuf,
    uid: Uid,
}

/// An open trash directory.
pub(super) struct Bin {
    files: OwnedFd,
    info: OwnedFd,
}

impl Bins {
    pub(super) fn new(data_home: &Path, uid: Uid) -> Self {
        Self {
            home: canonical(data_home).join("Trash"),
            uid,
        }
    }

    /// The trash for an item on filesystem `dev` inside the canonical directory `parent`.
    /// In a dry run, `None` means the trash doesn't exist yet and would be created.
    /// `Err` means there is no usable trash.
    pub(super) fn open(
        &self,
        dev: u64,
        parent: &Path,
        execution: Execution,
    ) -> Result<Option<Bin>, Errno> {
        let root = if nearest_dev(&self.home) == Some(dev) {
            self.home.clone()
        } else {
            topdir(parent, dev).join(format!(".Trash-{}", self.uid.as_raw()))
        };
        Bin::open(&root, self.uid, execution)
    }
}

impl Bin {
    fn open(root: &Path, uid: Uid, execution: Execution) -> Result<Option<Self>, Errno> {
        let (Some(parent), Some(name)) = (root.parent(), root.file_name()) else {
            return Err(Errno::INVAL);
        };
        if execution == Execution::Live {
            std::fs::create_dir_all(parent)
                .map_err(|e| Errno::from_io_error(&e).unwrap_or(Errno::IO))?;
        }
        let parent = match open_dir_nofollow(parent) {
            Err(Errno::NOENT) => return Ok(None),
            other => other?,
        };
        let Some(bin) = open_or_create(&parent, name, execution)? else {
            return Ok(None);
        };
        // A trash someone else owns would hand them the user's files.
        if fstat(&bin)?.st_uid != uid.as_raw() {
            return Err(Errno::PERM);
        }
        let files = open_or_create(&bin, "files", execution)?;
        let info = open_or_create(&bin, "info", execution)?;
        Ok(files.zip(info).map(|(files, info)| Self { files, info }))
    }

    /// Moves `name` (inside `parent_fd`, at canonical `path`) into the trash. The info
    /// file is created first because `O_EXCL` reserves the name; the rename then never
    /// replaces anything, so a name taken in `files/` alone is also skipped.
    pub(super) fn put(&self, parent_fd: &OwnedFd, name: &str, path: &Path) -> Result<(), Errno> {
        let info = trash_info(path);
        for n in 1..=MAX_NAMES {
            let trashed = match n {
                1 => name.to_owned(),
                n => format!("{name}.{n}"),
            };
            let info_name = format!("{trashed}.trashinfo");
            let file = match openat(
                &self.info,
                &info_name,
                NEW_INFO,
                FileMode::RUSR | FileMode::WUSR,
            ) {
                Ok(fd) => File::from(fd),
                Err(Errno::EXIST) => continue,
                Err(errno) => return Err(errno),
            };
            let moved = write_all(file, &info).and_then(|()| {
                renameat_with(
                    parent_fd,
                    name,
                    &self.files,
                    &trashed,
                    RenameFlags::NOREPLACE,
                )
            });
            match moved {
                Ok(()) => return Ok(()),
                Err(errno) => {
                    // Best effort: a stray .trashinfo without its item is harmless.
                    let _ = unlinkat(&self.info, &info_name, AtFlags::empty());
                    if errno != Errno::EXIST {
                        return Err(errno);
                    }
                }
            }
        }
        Err(Errno::EXIST)
    }
}

/// Opens directory `name` in `dir`, creating it (mode 0700) in a live run.
fn open_or_create(
    dir: &OwnedFd,
    name: impl rustix::path::Arg + Copy,
    execution: Execution,
) -> Result<Option<OwnedFd>, Errno> {
    match openat(dir, name, WALK, FileMode::empty()) {
        Err(Errno::NOENT) if execution == Execution::DryRun => Ok(None),
        Err(Errno::NOENT) => {
            match mkdirat(dir, name, FileMode::RWXU) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(errno) => return Err(errno),
            }
            openat(dir, name, WALK, FileMode::empty()).map(Some)
        }
        other => other.map(Some),
    }
}

fn write_all(mut file: File, contents: &str) -> Result<(), Errno> {
    file.write_all(contents.as_bytes())
        .map_err(|e| Errno::from_io_error(&e).unwrap_or(Errno::IO))
}

/// The filesystem of `path`, or of its nearest existing ancestor.
fn nearest_dev(path: &Path) -> Option<u64> {
    path.ancestors()
        .find_map(|dir| stat(dir).ok())
        .map(|st| Identity::from(&st).dev)
}

/// The mount point holding the canonical directory `dir`: its highest ancestor on `dev`.
fn topdir(dir: &Path, dev: u64) -> &Path {
    dir.ancestors()
        .take_while(|up| stat(*up).is_ok_and(|st| Identity::from(&st).dev == dev))
        .last()
        .unwrap_or(dir)
}

fn trash_info(path: &Path) -> String {
    let date = jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:%S");
    format!(
        "[Trash Info]\nPath={}\nDeletionDate={date}\n",
        percent_encode(path.as_os_str().as_encoded_bytes())
    )
}

/// URL-encodes a path for `Path=`, keeping `/` and the unreserved characters.
fn percent_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

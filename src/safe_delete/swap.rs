//! SE-05: one file put in place of another in a single exchange, so the target is never
//! missing or half-written. The only rename outside the trash (P-01).

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use rustix::fs::{
    AtFlags, CWD, FileType, Mode as FileMode, OFlags, RenameFlags, Uid, fchmod, fstat, openat,
    renameat_with, statat, unlinkat,
};
use rustix::io::Errno;

#[cfg(test)]
mod tests;

fn errno(error: &io::Error) -> Errno {
    Errno::from_io_error(error).unwrap_or(Errno::IO)
}

/// Copies `new` beside `target` (mode 0755) and exchanges the two in one
/// `renameat2(RENAME_EXCHANGE)`. Gives the path the old file has now, for the caller to
/// trash. `target` must be a regular file `euid` owns, in a folder `euid` owns; no
/// symlink is followed. On any failure nothing is replaced and the copy is removed.
pub fn swap_in(target: &Path, new: &Path, euid: Uid) -> Result<PathBuf, Errno> {
    #[cfg(test)]
    super::tests::assert_in_temp(target);
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(Errno::INVAL);
    };
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let dir = openat(CWD, parent, flags | OFlags::DIRECTORY, FileMode::empty())?;
    let old = statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW)?;
    let ours = |uid: u32| Uid::from_raw(uid) == euid;
    if FileType::from_raw_mode(old.st_mode) != FileType::RegularFile {
        return Err(Errno::INVAL);
    }
    if !ours(old.st_uid) || !ours(fstat(&dir)?.st_uid) {
        return Err(Errno::PERM);
    }
    let source = openat(CWD, new, flags, FileMode::empty())?;
    if FileType::from_raw_mode(fstat(&source)?.st_mode) != FileType::RegularFile {
        return Err(Errno::INVAL);
    }

    let temp = format!(
        ".{}.molt-update-{}",
        name.to_string_lossy(),
        std::process::id()
    );
    let copy = openat(
        &dir,
        temp.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        FileMode::from_raw_mode(0o755),
    )?;
    let written = (|| {
        // The umask may have narrowed the mode `openat` was given.
        fchmod(&copy, FileMode::from_raw_mode(0o755))?;
        let mut copy = File::from(copy);
        io::copy(&mut File::from(source), &mut copy).map_err(|e| errno(&e))?;
        copy.sync_all().map_err(|e| errno(&e))?;
        renameat_with(&dir, temp.as_str(), &dir, name, RenameFlags::EXCHANGE)
    })();
    match written {
        Ok(()) => Ok(parent.join(temp)),
        Err(error) => {
            // Best effort: the copy isn't in place, so it's only a stray file.
            let _ = unlinkat(&dir, temp.as_str(), AtFlags::empty());
            Err(error)
        }
    }
}

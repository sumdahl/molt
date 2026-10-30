//! Tests for the swap (SE-05), in a temp folder.

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

use super::*;

fn euid() -> Uid {
    rustix::process::geteuid()
}

#[test]
fn se_05_the_new_file_takes_the_place_and_the_old_one_moves_aside() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("molt");
    let new = dir.path().join("new");
    fs::write(&target, "old").unwrap();
    fs::write(&new, "new").unwrap();
    let inode = fs::metadata(&target).unwrap().ino();

    let old = swap_in(&target, &new, euid()).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "new");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(fs::read_to_string(&old).unwrap(), "old");
    assert_eq!(
        fs::metadata(&old).unwrap().ino(),
        inode,
        "the old file itself, moved"
    );
    assert_eq!(old.parent(), target.parent());
}

#[test]
fn se_04_se_05_links_folders_and_other_owners_are_refused_and_nothing_is_left() {
    let dir = tempfile::tempdir().unwrap();
    let new = dir.path().join("new");
    fs::write(&new, "new").unwrap();
    let real = dir.path().join("real");
    fs::write(&real, "old").unwrap();
    let link = dir.path().join("link");
    symlink(&real, &link).unwrap();
    fs::create_dir(dir.path().join("folder")).unwrap();

    assert_eq!(swap_in(&link, &new, euid()), Err(Errno::INVAL));
    assert_eq!(
        swap_in(&dir.path().join("folder"), &new, euid()),
        Err(Errno::INVAL)
    );
    let someone = Uid::from_raw(euid().as_raw() + 1);
    assert_eq!(swap_in(&real, &new, someone), Err(Errno::PERM));
    assert_eq!(
        swap_in(&real, &link, euid()),
        Err(Errno::LOOP),
        "the new file can't be a link"
    );
    assert_eq!(fs::read_to_string(&real).unwrap(), "old");
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 4, "no copy left behind: {names:?}");
}

//! Tests for the analyze walk (specs/012-analyze.md), on temp folders only.

use super::*;

use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

use tempfile::TempDir;

use crate::safe_delete::tests::{in_mount_namespace, mount_tmpfs};

fn write(path: &Path, len: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![b'x'; len]).unwrap();
}

fn on_disk(path: &Path) -> u64 {
    fs::symlink_metadata(path).unwrap().blocks() * 512
}

fn child<'a>(node: &'a Node, name: &str) -> &'a Node {
    node.children
        .iter()
        .find(|child| child.name == name)
        .unwrap_or_else(|| panic!("no {name} in {:?}", node.children))
}

#[test]
fn an_01_sizes_are_on_disk_and_add_up() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(&root.join("a/big"), 64 * 1024);
    write(&root.join("a/small"), 10);
    write(&root.join("top"), 4096);

    let tree = measure(root, 2).unwrap();

    let a = child(&tree, "a");
    assert_eq!(a.kind, EntryKind::Dir);
    assert_eq!(child(a, "big").bytes, on_disk(&root.join("a/big")));
    assert_eq!(
        a.bytes,
        on_disk(&root.join("a")) + on_disk(&root.join("a/big")) + on_disk(&root.join("a/small"))
    );
    assert_eq!(
        tree.bytes,
        on_disk(root) + a.bytes + on_disk(&root.join("top"))
    );
    assert!(!tree.partial);
}

#[test]
fn an_01_a_hard_link_pair_counts_once_for_the_first_path_by_name() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(&root.join("a/file"), 64 * 1024);
    fs::create_dir(root.join("b")).unwrap();
    fs::hard_link(root.join("a/file"), root.join("b/link")).unwrap();
    let file = on_disk(&root.join("a/file"));

    let tree = measure(root, 2).unwrap();

    assert_eq!(child(child(&tree, "a"), "file").bytes, file);
    assert_eq!(child(child(&tree, "b"), "link").bytes, 0);
    assert_eq!(child(&tree, "b").bytes, on_disk(&root.join("b")));
}

#[test]
fn an_02_a_symlink_counts_as_itself_and_is_not_followed() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    write(&outside.path().join("big"), 1024 * 1024);
    symlink(outside.path(), dir.path().join("link")).unwrap();

    let tree = measure(dir.path(), 1).unwrap();

    let link = child(&tree, "link");
    assert_eq!(link.kind, EntryKind::Symlink);
    assert_eq!(link.bytes, on_disk(&dir.path().join("link")));
    assert!(tree.bytes < 1024 * 1024);
}

#[test]
fn an_02_mounts_inside_are_listed_but_never_entered() {
    in_mount_namespace("an_02_mounts_inside", || {
        let dir = TempDir::new().unwrap();
        let mnt = dir.path().join("mnt");
        mount_tmpfs(&mnt);
        write(&mnt.join("data"), 1024 * 1024);

        let tree = measure(dir.path(), 2).unwrap();

        let mount = child(&tree, "mnt");
        assert_eq!(mount.kind, EntryKind::Mount);
        assert_eq!(mount.bytes, 0);
        assert!(mount.children.is_empty());
    });
}

#[test]
fn an_03_an_unreadable_folder_makes_its_parents_partial() {
    if rustix::process::geteuid().is_root() {
        return eprintln!("skipped: root reads any folder");
    }
    let dir = TempDir::new().unwrap();
    let locked = dir.path().join("a/locked");
    write(&locked.join("secret"), 4096);
    write(&dir.path().join("b/fine"), 10);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

    let tree = measure(dir.path(), 3);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    let tree = tree.unwrap();

    assert!(tree.partial);
    let a = child(&tree, "a");
    assert!(a.partial);
    assert!(child(a, "locked").partial);
    assert!(!child(&tree, "b").partial);
}

#[test]
fn an_04_the_result_does_not_depend_on_thread_timing() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    for i in 0..40 {
        write(&root.join(format!("d{i}/sub/f")), 4096 * (i % 5 + 1));
    }
    for i in 0..40 {
        if i % 3 == 0 {
            // Hard links across folders: which one counts must not depend on timing.
            fs::hard_link(
                root.join(format!("d{i}/sub/f")),
                root.join(format!("d{}/link{i}", (i + 7) % 40)),
            )
            .unwrap();
        }
    }

    let first = measure(root, 3).unwrap();
    for _ in 0..20 {
        assert_eq!(measure(root, 3).unwrap(), first);
    }
}

#[test]
fn an_05_the_path_must_be_a_directory() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("file");
    write(&file, 10);
    symlink(dir.path(), dir.path().join("link")).unwrap();

    assert!(matches!(
        measure(&file, 1),
        Err(AnalyzeError::NotADirectory(_))
    ));
    assert!(matches!(
        measure(&dir.path().join("link"), 1),
        Err(AnalyzeError::NotADirectory(_))
    ));
    assert!(matches!(
        measure(&dir.path().join("missing"), 1),
        Err(AnalyzeError::Unreadable(..))
    ));
}

#[test]
fn an_06_an_07_text_is_largest_first_and_cut_at_20() {
    let dir = TempDir::new().unwrap();
    for i in 0..25 {
        write(&dir.path().join(format!("f{i:02}")), 4096 * (i + 1));
    }
    write(&dir.path().join("d/x"), 10);

    let tree = measure(dir.path(), 1).unwrap();
    let text = Listing::new(&tree, 1, false).to_string();
    let lines: Vec<&str> = text.lines().collect();

    assert!(lines[1].ends_with("f24"), "{text}");
    assert!(lines[2].ends_with("f23"), "{text}");
    assert!(text.contains("… 6 more"), "{text}");
    assert_eq!(lines.len(), 1 + 20 + 1, "{text}");

    let all = Listing::new(&tree, 1, true).to_string();
    assert!(all.contains("d/"), "{all}");
    assert!(!all.contains("more"), "{all}");
}

#[test]
fn an_08_json_children_stop_at_the_depth() {
    let dir = TempDir::new().unwrap();
    write(&dir.path().join("a/b/c"), 10);

    let tree = measure(dir.path(), 1).unwrap();
    let json = serde_json::to_value(&tree).unwrap();

    let a = &json["children"][0];
    assert_eq!(a["name"], "a");
    assert_eq!(a["kind"], "dir");
    assert_eq!(a["partial"], false);
    assert!(a["children"].as_array().unwrap().is_empty());
}

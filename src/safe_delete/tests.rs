//! Tests for specs/001-safe-delete.md. Each test names the requirement it proves.
//!
//! Every test runs inside a temporary directory that stands in for `$HOME`. Tests that
//! name real system paths use `Execution::DryRun`, so a guard bug can't touch the system.

use super::*;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::process::Command;
use std::time::{Duration, SystemTime};

use proptest::prelude::*;
use tempfile::TempDir;

type Hook = Box<dyn FnMut(&CStr)>;

thread_local! {
    static BEFORE_DESCEND: RefCell<Option<Hook>> = const { RefCell::new(None) };
}

/// Tripwire for every plan run in a test build: a live plan may only touch paths inside
/// the system temp directory. A buggy test then panics instead of deleting real files.
pub(super) fn assert_sandboxed(plan: &Plan) {
    if plan.execution == Execution::DryRun {
        return;
    }
    let temp = fs::canonicalize(std::env::temp_dir()).unwrap();
    for candidate in &plan.candidates {
        let parent = candidate
            .path()
            .parent()
            .and_then(|p| fs::canonicalize(p).ok());
        assert!(
            parent.is_some_and(|p| p.starts_with(&temp) && p != temp),
            "test tried a live plan outside the temp dir: {:?}",
            candidate.path()
        );
    }
}

/// Called by the removal walk before it opens each subdirectory (SD-09 race tests).
pub(super) fn before_descend(name: &CStr) {
    BEFORE_DESCEND.with_borrow_mut(|hook| {
        if let Some(hook) = hook {
            hook(name);
        }
    });
}

/// A fake home with XDG dirs inside it, next to an `outside` dir that must never change.
struct Sandbox {
    _dir: TempDir,
    home: PathBuf,
    outside: PathBuf,
    ctx: Context,
}

impl Sandbox {
    fn new() -> Self {
        Self::with_whitelist(&[])
    }

    fn with_whitelist(whitelist: &[&str]) -> Self {
        let dir = TempDir::new().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let home = root.join("home");
        let outside = root.join("outside");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(outside.join("dir")).unwrap();
        fs::write(outside.join("file"), "precious").unwrap();
        fs::write(outside.join("dir/precious"), "precious").unwrap();
        let ctx = Context {
            data_home: home.join(".local/share"),
            xdg_dirs: [".config", ".cache", ".local/share", ".local/state"]
                .map(|d| home.join(d))
                .to_vec(),
            whitelist: whitelist.iter().map(|w| home.join(w)).collect(),
            home: home.clone(),
            // The owner of a fresh temp dir is the effective uid (root inside a test namespace).
            euid: Uid::from_raw(fs::metadata(&root).unwrap().uid()),
        };
        Self {
            _dir: dir,
            home,
            outside,
            ctx,
        }
    }

    /// Creates a file (and its parents) under home and returns its path.
    fn file(&self, rel: &str) -> PathBuf {
        let path = self.home.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![b'x'; 64 * 1024]).unwrap();
        path
    }

    fn plan(&self, paths: &[&Path], execution: Execution) -> Plan {
        self.plan_with(paths, Mode::Permanent, execution)
    }

    fn plan_with(&self, paths: &[&Path], mode: Mode, execution: Execution) -> Plan {
        // SD-16 would refuse every plan inside a test user namespace, where we are root.
        let privilege = if self.ctx.euid.is_root() {
            Privilege::System
        } else {
            Privilege::User
        };
        Plan {
            candidates: paths.iter().map(|p| Candidate::scan(p).unwrap()).collect(),
            mode,
            execution,
            privilege,
        }
    }

    fn run(&self, paths: &[&Path], execution: Execution) -> Vec<Outcome> {
        self.run_with(paths, Mode::Permanent, execution)
    }

    fn run_with(&self, paths: &[&Path], mode: Mode, execution: Execution) -> Vec<Outcome> {
        execute(&self.plan_with(paths, mode, execution), &self.ctx)
            .unwrap()
            .results
            .into_iter()
            .map(|(_, outcome)| outcome)
            .collect()
    }
}

/// Every path under `root` with its inode and size, without following symlinks.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (u64, u64)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            for entry in fs::read_dir(&path).unwrap() {
                stack.push(entry.unwrap().path());
            }
        }
        out.insert(path, (meta.ino(), meta.size()));
    }
    out
}

fn on_disk(path: &Path) -> u64 {
    fs::symlink_metadata(path).unwrap().blocks() * 512
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn mv(from: &Path, to: &Path) {
    assert!(
        Command::new("mv")
            .arg(from)
            .arg(to)
            .status()
            .unwrap()
            .success()
    );
}

fn removed() -> impl Fn(&Outcome) -> bool {
    |o| matches!(o, Outcome::Removed { partial: false, .. })
}

// Test safety

#[test]
#[should_panic(expected = "outside the temp dir")]
fn tests_cannot_run_a_live_plan_outside_the_temp_dir() {
    let sb = Sandbox::new();
    // procfs entries can't be unlinked, so even a broken tripwire removes nothing.
    sb.run(&[Path::new("/proc/self/status")], Execution::Live);
}

// Refusals

#[test]
fn sd_01_root_home_xdg_and_top_level_dirs_are_protected() {
    let sb = Sandbox::new();
    let above_home = sb.home.parent().unwrap();
    let cache = sb.home.join(".cache");
    fs::create_dir_all(&cache).unwrap();
    let paths = [
        Path::new("/"),
        Path::new("/usr"),
        Path::new("/etc"),
        &sb.home,
        &cache,
        above_home,
    ];

    for (path, outcome) in paths.iter().zip(sb.run(&paths, Execution::DryRun)) {
        assert_eq!(outcome, Outcome::Skipped(Reason::Protected), "{path:?}");
    }
}

#[test]
fn sd_02_secrets_and_directories_above_them_are_protected() {
    let sb = Sandbox::new();
    let secrets = [
        ".ssh/id_ed25519",
        ".gnupg/pubring.kbx",
        ".password-store/mail.gpg",
        ".local/share/keyrings/login.keyring",
        ".mozilla/firefox/abc.default/key4.db",
        ".mozilla/firefox/abc.default/logins.json",
    ]
    .map(|s| sb.file(s));
    let above = [
        ".ssh",
        ".mozilla",
        ".mozilla/firefox",
        ".mozilla/firefox/abc.default",
    ]
    .map(|d| sb.home.join(d));
    let paths: Vec<&Path> = secrets.iter().chain(&above).map(PathBuf::as_path).collect();

    for (path, outcome) in paths.iter().zip(sb.run(&paths, Execution::Live)) {
        assert_eq!(outcome, Outcome::Skipped(Reason::Protected), "{path:?}");
    }
    assert!(secrets.iter().all(|s| exists(s)));
}

#[test]
fn sd_02_firefox_profile_cache_is_not_protected() {
    let sb = Sandbox::new();
    let cache = sb.file(".mozilla/firefox/abc.default/cache2/entry");
    let cache = cache.parent().unwrap();

    assert!(removed()(&sb.run(&[cache], Execution::Live)[0]));
    assert!(!exists(cache));
}

#[test]
fn sd_03_whitelisted_paths_their_contents_and_parents_are_refused() {
    let sb = Sandbox::with_whitelist(&["proj/keep"]);
    let inside = sb.file("proj/keep/notes");
    let other = sb.file("proj/other");
    let keep = sb.home.join("proj/keep");
    let proj = sb.home.join("proj");

    let outcomes = sb.run(&[&keep, &inside, &proj, &other], Execution::Live);

    assert_eq!(&outcomes[..3], &[Outcome::Skipped(Reason::Whitelisted); 3]);
    assert!(removed()(&outcomes[3]));
    assert!(exists(&inside));
}

#[test]
fn sd_04_relative_dotdot_and_non_utf8_paths_are_refused() {
    let sb = Sandbox::new();
    sb.file("a/f");
    let dotdot = sb.home.join("a/../a/f");
    let non_utf8 = sb.home.join(OsStr::from_bytes(b"caf\xe9"));
    fs::write(&non_utf8, "").unwrap();
    let paths = [Path::new("Cargo.toml"), &dotdot, &non_utf8];

    for (path, outcome) in paths.iter().zip(sb.run(&paths, Execution::DryRun)) {
        assert_eq!(outcome, Outcome::Skipped(Reason::InvalidPath), "{path:?}");
    }
}

// Symlinks and mounts

#[test]
fn sd_05_only_the_symlink_is_removed_never_its_target() {
    let sb = Sandbox::new();
    let link = sb.home.join("link");
    symlink(sb.outside.join("file"), &link).unwrap();
    let before = snapshot(&sb.outside);

    assert!(removed()(&sb.run(&[&link], Execution::Live)[0]));
    assert!(!exists(&link));
    assert_eq!(snapshot(&sb.outside), before);
}

#[test]
fn sd_06_symlinks_inside_a_directory_are_removed_as_links() {
    let sb = Sandbox::new();
    let app = sb.file(".cache/app/junk");
    let app = app.parent().unwrap();
    symlink(sb.outside.join("file"), app.join("to-file")).unwrap();
    symlink(sb.outside.join("dir"), app.join("to-dir")).unwrap();
    let before = snapshot(&sb.outside);

    assert!(removed()(&sb.run(&[app], Execution::Live)[0]));
    assert!(!exists(app));
    assert_eq!(snapshot(&sb.outside), before);
}

/// Creates `at` and mounts a fresh tmpfs on it. Only call inside `in_mount_namespace`.
fn mount_tmpfs(at: &Path) {
    fs::create_dir(at).unwrap();
    let status = Command::new("mount")
        .args(["-t", "tmpfs", "tmpfs"])
        .arg(at)
        .status()
        .unwrap();
    assert!(status.success());
}

/// Runs `body` in a private user and mount namespace so the test can mount a tmpfs
/// without root. Locally it skips (with a note) where unprivileged user namespaces are
/// disabled; on CI it fails instead.
fn in_mount_namespace(filter: &str, body: impl FnOnce()) {
    if std::env::var_os("MOLT_TEST_IN_NS").is_some() {
        return body();
    }
    let unshare = || {
        let mut cmd = Command::new("unshare");
        cmd.args(["--user", "--map-root-user", "--mount", "--"]);
        cmd
    };
    let available = unshare().arg("true").status().is_ok_and(|s| s.success());
    if !available {
        // A safety test that silently skips proves nothing, so CI must be able to run it.
        assert!(
            std::env::var_os("CI").is_none(),
            "{filter} needs unprivileged user namespaces on CI"
        );
        eprintln!("skipped {filter}: unprivileged user namespaces unavailable");
        return;
    }
    let status = unshare()
        .arg(std::env::current_exe().unwrap())
        .args([filter, "--test-threads=1"])
        .env("MOLT_TEST_IN_NS", "1")
        .status()
        .unwrap();
    assert!(status.success(), "{filter} failed inside the namespace");
}

#[test]
fn sd_07_mounts_inside_a_candidate_are_never_entered() {
    in_mount_namespace("sd_07_mounts_inside", || {
        let sb = Sandbox::new();
        let junk = sb.file(".cache/app/junk");
        let app = junk.parent().unwrap();
        let mnt = app.join("mnt");
        mount_tmpfs(&mnt);
        fs::write(mnt.join("data"), "on another filesystem").unwrap();

        let outcome = sb.run(&[app], Execution::Live).remove(0);

        assert!(matches!(outcome, Outcome::Removed { partial: true, .. }));
        assert!(!exists(&junk));
        assert!(exists(&mnt.join("data")));
        assert_eq!(
            sb.run(&[&mnt], Execution::Live),
            [Outcome::Skipped(Reason::MountPoint)]
        );
    });
}

// Races

#[test]
fn sd_08_a_replaced_or_vanished_candidate_is_skipped() {
    let sb = Sandbox::new();
    let replaced = sb.file("replaced");
    let vanished = sb.file("vanished");
    let plan = sb.plan(&[&replaced, &vanished], Execution::Live);
    mv(&sb.file("impostor"), &replaced);
    mv(&vanished, &sb.home.join("elsewhere"));

    let report = execute(&plan, &sb.ctx).unwrap();

    assert!(
        report
            .results
            .iter()
            .all(|(_, o)| *o == Outcome::Skipped(Reason::ChangedSinceScan))
    );
    assert!(exists(&replaced));
}

#[test]
fn sd_09_a_directory_swapped_for_a_symlink_mid_removal_cannot_redirect_it() {
    let sb = Sandbox::new();
    sb.file("cache/sub/file");
    let cache = sb.home.join("cache");
    let sub = cache.join("sub");
    let moved = sb.home.join("moved");
    let target = sb.outside.clone();
    let before = snapshot(&sb.outside);
    let plan = sb.plan(&[&cache], Execution::Live);
    let (sub_hook, moved_hook) = (sub.clone(), moved.clone());
    BEFORE_DESCEND.set(Some(Box::new(move |name| {
        if name == c"sub" {
            rustix::fs::renameat(CWD, &sub_hook, CWD, &moved_hook).unwrap();
            symlink(&target, &sub_hook).unwrap();
        }
    })));

    let outcome = execute(&plan, &sb.ctx).unwrap().results.remove(0).1;
    BEFORE_DESCEND.set(None);

    assert!(matches!(outcome, Outcome::Failed { .. }), "{outcome:?}");
    assert_eq!(snapshot(&sb.outside), before);
    assert!(exists(&moved.join("file")));
}

#[test]
fn sd_10_a_candidate_modified_after_the_scan_is_skipped() {
    let sb = Sandbox::new();
    let file = sb.file("busy.log");
    let plan = sb.plan(&[&file], Execution::Live);
    fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(60))
        .unwrap();

    let outcome = execute(&plan, &sb.ctx).unwrap().results.remove(0).1;

    assert_eq!(outcome, Outcome::Skipped(Reason::ModifiedSinceScan));
    assert!(exists(&file));
}

// Modes

#[test]
fn sd_12_dry_run_changes_nothing_and_reports_what_a_live_run_does() {
    let sb = Sandbox::new();
    sb.file("cache/a/one");
    sb.file("cache/b/two");
    let link = sb.home.join("cache/link");
    symlink(sb.outside.join("file"), &link).unwrap();
    let cache = sb.home.join("cache");
    let before = snapshot(&sb.home);

    let dry = sb.run(&[&cache], Execution::DryRun);
    assert_eq!(snapshot(&sb.home), before);
    let live = sb.run(&[&cache], Execution::Live);

    let Outcome::WouldRemove { bytes, partial } = dry[0] else {
        panic!("{dry:?}")
    };
    assert_eq!(live, [Outcome::Removed { bytes, partial }]);
    assert!(bytes > 0);
}

#[test]
fn sd_11_items_go_to_the_home_trash_with_a_restorable_trashinfo() {
    let sb = Sandbox::new();
    let file = sb.file("Downloads/my file.iso");
    let dir = sb
        .file("Downloads/old app/bin")
        .parent()
        .unwrap()
        .to_path_buf();
    let trash = sb.home.join(".local/share/Trash");

    let outcomes = sb.run_with(&[&file, &dir], Mode::Trash, Execution::Live);

    assert!(
        outcomes
            .iter()
            .all(|o| matches!(o, Outcome::Trashed { bytes } if *bytes > 0))
    );
    assert!(!exists(&file) && !exists(&dir));
    assert!(exists(&trash.join("files/my file.iso")));
    assert!(exists(&trash.join("files/old app/bin")));
    let info = fs::read_to_string(trash.join("info/my file.iso.trashinfo")).unwrap();
    let encoded = file.to_str().unwrap().replace(' ', "%20");
    let mut lines = info.lines();
    assert_eq!(lines.next(), Some("[Trash Info]"));
    assert_eq!(lines.next(), Some(format!("Path={encoded}").as_str()));
    let date = lines.next().unwrap().strip_prefix("DeletionDate=").unwrap();
    assert!(
        jiff::civil::DateTime::strptime("%Y-%m-%dT%H:%M:%S", date).is_ok(),
        "{date}"
    );
}

#[test]
fn sd_11_same_names_never_overwrite_each_other_in_the_trash() {
    let sb = Sandbox::new();
    let a = sb.file("a/report.pdf");
    let b = sb.file("b/report.pdf");
    let trash = sb.home.join(".local/share/Trash");

    sb.run_with(&[&a, &b], Mode::Trash, Execution::Live);

    for name in ["report.pdf", "report.pdf.2"] {
        assert!(exists(&trash.join("files").join(name)), "{name}");
        assert!(
            exists(&trash.join("info").join(format!("{name}.trashinfo"))),
            "{name}"
        );
    }
}

#[test]
fn sd_11_sd_12_a_trash_dry_run_creates_nothing() {
    let sb = Sandbox::new();
    let file = sb.file("Downloads/setup.run");
    let before = snapshot(&sb.home);

    let dry = sb.run_with(&[&file], Mode::Trash, Execution::DryRun);
    assert_eq!(snapshot(&sb.home), before);
    let live = sb.run_with(&[&file], Mode::Trash, Execution::Live);

    let Outcome::WouldTrash { bytes } = dry[0] else {
        panic!("{dry:?}")
    };
    assert_eq!(live, [Outcome::Trashed { bytes }]);
}

#[test]
fn sd_11_items_on_other_filesystems_go_to_the_topdir_trash() {
    in_mount_namespace("sd_11_items_on_other", || {
        let sb = Sandbox::new();
        let mnt = sb.home.parent().unwrap().join("mnt");
        mount_tmpfs(&mnt);
        let file = mnt.join("disk.qcow2");
        fs::write(&file, "vm").unwrap();

        let outcome = sb.run_with(&[&file], Mode::Trash, Execution::Live);

        assert!(matches!(outcome[0], Outcome::Trashed { .. }), "{outcome:?}");
        let bin = mnt.join(format!(".Trash-{}", sb.ctx.euid.as_raw()));
        assert!(exists(&bin.join("files/disk.qcow2")));
        assert!(exists(&bin.join("info/disk.qcow2.trashinfo")));
    });
}

#[test]
fn sd_11_without_a_usable_trash_items_are_skipped_never_deleted() {
    in_mount_namespace("sd_11_without_a_usable", || {
        let sb = Sandbox::new();
        let mnt = sb.home.parent().unwrap().join("mnt");
        mount_tmpfs(&mnt);
        let file = mnt.join("keep.iso");
        fs::write(&file, "data").unwrap();
        // A trash that is a symlink could send the item anywhere.
        symlink(
            &sb.outside,
            mnt.join(format!(".Trash-{}", sb.ctx.euid.as_raw())),
        )
        .unwrap();
        let before = snapshot(&sb.outside);

        let outcome = sb.run_with(&[&file], Mode::Trash, Execution::Live);

        assert_eq!(outcome, [Outcome::Skipped(Reason::NoTrash)]);
        assert!(exists(&file));
        assert_eq!(snapshot(&sb.outside), before);
    });
}

#[test]
fn sd_07_sd_11_a_directory_holding_a_mount_is_not_trashed() {
    in_mount_namespace("sd_07_sd_11_a_directory", || {
        let sb = Sandbox::new();
        let app = sb
            .file("Downloads/app/junk")
            .parent()
            .unwrap()
            .to_path_buf();
        let mnt = app.join("mnt");
        mount_tmpfs(&mnt);
        fs::write(mnt.join("data"), "on another filesystem").unwrap();

        let outcome = sb.run_with(&[&app], Mode::Trash, Execution::Live);

        assert_eq!(outcome, [Outcome::Skipped(Reason::MountPoint)]);
        assert!(exists(&mnt.join("data")));
    });
}

// Reporting

#[test]
fn sd_13_hard_links_count_once_and_only_when_the_last_link_goes() {
    let sb = Sandbox::new();
    let both = sb.file("both/a");
    fs::hard_link(&both, sb.home.join("both/b")).unwrap();
    let half = sb.file("half/a");
    fs::hard_link(&half, sb.outside.join("half-link")).unwrap();
    let (both_dir, half_dir) = (sb.home.join("both"), sb.home.join("half"));
    let expected = [on_disk(&both_dir) + on_disk(&both), on_disk(&half_dir)];

    for execution in [Execution::DryRun, Execution::Live] {
        let bytes: Vec<u64> = sb
            .run(&[&both_dir, &half_dir], execution)
            .iter()
            .map(|o| match *o {
                Outcome::WouldRemove { bytes, .. } | Outcome::Removed { bytes, .. } => bytes,
                ref other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(bytes, expected, "{execution:?}");
    }
}

#[test]
fn sd_14_one_failure_does_not_stop_the_plan_and_exits_3() {
    let sb = Sandbox::new();
    let locked = sb.file("locked/sub/file");
    if fs::metadata(&locked).unwrap().uid() == 0 {
        eprintln!("skipped: permissions are not enforced for root");
        return;
    }
    let sub = locked.parent().unwrap();
    fs::set_permissions(sub, fs::Permissions::from_mode(0o555)).unwrap();
    let ok = sb.file("ok");
    let locked_dir = sb.home.join("locked");

    let report = execute(&sb.plan(&[&locked_dir, &ok], Execution::Live), &sb.ctx).unwrap();
    fs::set_permissions(sub, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(matches!(
        report.results[0].1,
        Outcome::Failed {
            errno: Errno::ACCESS,
            ..
        }
    ));
    assert!(removed()(&report.results[1].1));
    assert_eq!(report.exit_code(), 3);
    assert!(exists(&locked));
}

// Privilege

#[test]
fn sd_16_root_without_a_system_step_is_refused_before_anything_is_touched() {
    let mut sb = Sandbox::new();
    let file = sb.file("cache/f");
    sb.ctx.euid = Uid::ROOT;
    let mut plan = sb.plan(&[&file], Execution::Live);
    plan.privilege = Privilege::User;

    assert_eq!(execute(&plan, &sb.ctx).unwrap_err(), PlanRefused::Root);
    assert!(exists(&file));

    plan.privilege = Privilege::System;
    assert!(removed()(&execute(&plan, &sb.ctx).unwrap().results[0].1));
}

// Property: random trees that link out of the candidate never change anything outside it.

#[derive(Debug, Clone, Copy)]
enum Node {
    Dir,
    File,
    LinkToFile,
    LinkToDir,
    HardLinkToFile,
}

fn node() -> impl Strategy<Value = Node> {
    prop_oneof![
        Just(Node::Dir),
        Just(Node::File),
        Just(Node::LinkToFile),
        Just(Node::LinkToDir),
        Just(Node::HardLinkToFile),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn sd_05_sd_06_nothing_outside_the_candidate_changes(
        shape in prop::collection::vec((node(), any::<prop::sample::Index>()), 1..40)
    ) {
        let sb = Sandbox::new();
        let tree = sb.home.join("tree");
        fs::create_dir(&tree).unwrap();
        let mut dirs = vec![tree.clone()];
        for (i, (node, parent)) in shape.into_iter().enumerate() {
            let path = parent.get(&dirs).join(format!("n{i}"));
            match node {
                Node::Dir => {
                    fs::create_dir(&path).unwrap();
                    dirs.push(path);
                }
                Node::File => fs::write(&path, "junk").unwrap(),
                Node::LinkToFile => symlink(sb.outside.join("file"), &path).unwrap(),
                Node::LinkToDir => symlink(sb.outside.join("dir"), &path).unwrap(),
                Node::HardLinkToFile => fs::hard_link(sb.outside.join("file"), &path).unwrap(),
            }
        }
        let before = snapshot(&sb.outside);

        let outcome = sb.run(&[&tree], Execution::Live);

        prop_assert!(removed()(&outcome[0]), "{:?}", outcome);
        prop_assert!(!exists(&tree));
        prop_assert_eq!(snapshot(&sb.outside), before);
    }
}

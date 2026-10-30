//! Tests for the system step (specs/007-system.md). Every tool is a fake script in a temp
//! folder, and every size is measured under a temp root, so nothing reads or runs the
//! real system's tools.

use super::*;

use std::os::unix::fs::PermissionsExt;

use tempfile::TempDir;

/// Writes an executable script `name` in `bin`.
fn script(bin: &Path, name: &str, body: &str) {
    fs::create_dir_all(bin).unwrap();
    let path = bin.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A fake `sudo` that drops the `--` and runs the rest, and the three tasks' tools.
fn all_tools(bin: &Path) {
    script(
        bin,
        "sudo",
        "[ \"$1\" = -- ] || exit 99\nshift\nexec \"$@\"",
    );
    for tool in ["paccache", "journalctl", "systemd-tmpfiles"] {
        script(bin, tool, "exit 0");
    }
}

/// Makes `name` not executable, so it isn't found as a tool.
fn hide(bin: &Path, name: &str) {
    fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn sy_02_tasks_are_active_only_when_their_tools_and_sudo_are_found() {
    let dir = TempDir::new().unwrap();
    let bin = dir.path().join("bin");
    all_tools(&bin);
    hide(&bin, "journalctl");

    let tasks = detect(std::slice::from_ref(&bin));
    let active: Vec<TaskId> = tasks.active.iter().map(|task| task.id).collect();
    assert_eq!(active, [TaskId::PacmanCache, TaskId::Coredumps]);
    assert_eq!(tasks.inactive, [TaskId::Journal]);

    hide(&bin, "sudo");
    let tasks = detect(&[bin]);
    assert!(tasks.active.is_empty(), "no sudo, no tasks");
    assert_eq!(tasks.inactive, TaskId::ALL);
}

#[test]
fn sy_01_only_and_skip_narrow_the_tasks_and_unknown_ids_are_refused() {
    let dir = TempDir::new().unwrap();
    let bin = dir.path().join("bin");
    all_tools(&bin);
    let active = detect(&[bin]).active;
    let ids = |tasks: Vec<Task>| tasks.iter().map(|task| task.id).collect::<Vec<_>>();

    let only = select(active.clone(), &["journal".into()], &[]).unwrap();
    assert_eq!(ids(only), [TaskId::Journal]);
    let skip = select(active.clone(), &[], &["journal".into()]).unwrap();
    assert_eq!(ids(skip), [TaskId::PacmanCache, TaskId::Coredumps]);
    assert_eq!(
        select(active, &["pacman".into()], &[]).unwrap_err(),
        "pacman"
    );
}

#[test]
fn sy_03_sy_07_the_preview_shows_the_exact_sudo_command_lines() {
    let dir = TempDir::new().unwrap();
    let bin = dir.path().join("bin");
    all_tools(&bin);
    script(
        &bin,
        "paccache",
        "echo '==> no candidate packages found for pruning'\necho\necho '==> finished dry run: 3 candidates (disk space saved: 1.50 MiB)'",
    );
    let tasks = detect(std::slice::from_ref(&bin));

    let preview = preview(
        &tasks,
        &tasks.active,
        dir.path(),
        std::slice::from_ref(&bin),
    );
    let pacman = &preview.tasks[0];
    assert_eq!(pacman.id, TaskId::PacmanCache);
    assert_eq!(
        pacman.would_run,
        [
            format!("{0}/sudo -- {0}/paccache -rk2", bin.display()),
            format!("{0}/sudo -- {0}/paccache -ruk0", bin.display()),
        ]
    );
    // SY-03 (#82): every non-empty line, not only the last.
    assert_eq!(
        pacman.previews[0].lines,
        [
            "==> no candidate packages found for pruning",
            "==> finished dry run: 3 candidates (disk space saved: 1.50 MiB)",
        ]
    );
    let shown = preview.to_string();
    assert!(
        shown.contains("preview: ==> no candidate packages"),
        "{shown}"
    );
    assert!(shown.contains("preview: ==> finished dry run"), "{shown}");
}

#[test]
fn sy_04_a_failed_preview_is_reported_and_the_task_still_shown() {
    let dir = TempDir::new().unwrap();
    let bin = dir.path().join("bin");
    all_tools(&bin);
    script(&bin, "journalctl", "exit 1");
    let tasks = detect(std::slice::from_ref(&bin));

    let preview = preview(&tasks, &tasks.active, dir.path(), &[bin]);
    let journal = &preview.tasks[1];
    assert_eq!(journal.previews[0].exit, Some(1));
    assert!(preview.to_string().contains("failed"));
}

#[test]
fn sy_08_sy_09_sy_11_commands_run_in_order_and_freed_is_before_minus_after() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    let pkg = root.join("var/cache/pacman/pkg");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join("old.pkg.tar.zst"), vec![b'x'; 64 * 1024]).unwrap();
    let bin = dir.path().join("bin");
    let ran = dir.path().join("ran");
    all_tools(&bin);
    // The fake paccache "cleans" the temp root, and every tool records its arguments.
    script(
        &bin,
        "paccache",
        &format!(
            "echo paccache \"$@\" >> {ran}\n[ \"$1\" = -rk2 ] && /bin/rm -f {pkg}/old.pkg.tar.zst\nexit 0",
            ran = ran.display(),
            pkg = pkg.display()
        ),
    );
    script(
        &bin,
        "journalctl",
        &format!("echo journalctl \"$@\" >> {}\nexit 1", ran.display()),
    );
    script(
        &bin,
        "systemd-tmpfiles",
        &format!("echo systemd-tmpfiles \"$@\" >> {}", ran.display()),
    );
    let tasks = detect(std::slice::from_ref(&bin));
    let sudo = tasks.sudo.clone().unwrap();
    let before = size(TaskId::PacmanCache, &root);
    assert!(before >= 64 * 1024);

    let oplog = dir.path().join("operations.log");
    let ctx = crate::safe_delete::Context {
        home: dir.path().to_owned(),
        data_home: dir.path().to_owned(),
        xdg_dirs: Vec::new(),
        whitelist: Vec::new(),
        oplog: Some(oplog.clone()),
        euid: rustix::process::geteuid(),
    };
    let mut log = CommandLog::open(&ctx).unwrap();
    let result = act(&sudo, &tasks.active, &root, &[bin], &mut log).unwrap();

    assert_eq!(
        fs::read_to_string(&ran).unwrap(),
        "paccache -rk2\npaccache -ruk0\njournalctl --vacuum-size=500M\n\
         systemd-tmpfiles --clean --prefix /var/lib/systemd/coredump\n",
        "the failed journal task didn't stop the coredumps task"
    );
    assert_eq!(
        result.tasks[0].freed,
        before - size(TaskId::PacmanCache, &root)
    );
    assert!(result.tasks[0].freed > 0);
    assert_eq!(result.tasks[1].commands[0].exit, Some(1));
    assert_eq!(result.exit_code(), 3);

    let log = fs::read_to_string(&oplog).unwrap();
    assert_eq!(log.lines().count(), 4, "{log}");
    assert!(
        log.contains(&format!("\"{}\",\"--\"", sudo.display())),
        "{log}"
    );
}

#[test]
fn sy_12_paccache_says_what_it_would_save() {
    let saved = |line| paccache_saved(line);
    assert_eq!(
        saved("==> no candidate packages found for pruning"),
        Some(0)
    );
    assert_eq!(
        saved("==> finished dry run: 22 candidates (disk space saved: 6.37 MiB)"),
        Some(6_679_429)
    );
    assert_eq!(saved("(disk space saved: 512 B)"), Some(512));
    assert_eq!(saved("(disk space saved: 2.00 KiB)"), Some(2048));
    assert_eq!(saved("(disk space saved: 1.00 GiB)"), Some(1 << 30));
    assert_eq!(saved("(disk space saved: 1.00 TiB)"), Some(1 << 40));
    for unknown in [
        "",
        "==> finished",
        "(disk space saved: lots)",
        "(disk space saved: 1 PiB)",
    ] {
        assert_eq!(saved(unknown), None, "{unknown}");
    }
}

fn output(lines: &[&str]) -> PreviewOutput {
    PreviewOutput {
        command: String::new(),
        exit: Some(0),
        lines: lines.iter().map(|line| (*line).to_owned()).collect(),
    }
}

#[test]
fn sy_12_would_free_per_task() {
    let root = TempDir::new().unwrap();
    let mib = 1024 * 1024;
    let pacman = [
        output(&["==> no candidate packages found for pruning"]),
        output(&["==> finished dry run: 1 candidates (disk space saved: 2.00 MiB)"]),
    ];
    assert_eq!(
        would_free(TaskId::PacmanCache, 0, &pacman, root.path()),
        WouldFree::Exactly(2 * mib)
    );
    let unknown = [output(&["something else"]), output(&[])];
    assert_eq!(
        would_free(TaskId::PacmanCache, 0, &unknown, root.path()),
        WouldFree::NotKnown
    );
    for (now, freed) in [(0, 0), (500 * mib, 0), (1500 * mib, 1000 * mib)] {
        assert_eq!(
            would_free(TaskId::Journal, now, &[], root.path()),
            WouldFree::About(freed),
            "{now}"
        );
    }
    assert_eq!(
        would_free(TaskId::Coredumps, 0, &[], root.path()),
        WouldFree::Exactly(0),
        "no folder"
    );
    let dumps = root.path().join("var/lib/systemd/coredump");
    fs::create_dir_all(&dumps).unwrap();
    assert_eq!(
        would_free(TaskId::Coredumps, 4096, &[], root.path()),
        WouldFree::Exactly(0),
        "empty"
    );
    fs::write(dumps.join("core.x.zst"), "dump").unwrap();
    assert_eq!(
        would_free(TaskId::Coredumps, 8192, &[], root.path()),
        WouldFree::NotKnown
    );
}

#[test]
fn sy_03_sy_13_now_would_free_and_the_total_are_shown() {
    let task = |name, bytes, would_free| TaskPreview {
        id: TaskId::Journal,
        name,
        bytes,
        would_free,
        previews: vec![output(&(0..12).map(|_| "line").collect::<Vec<_>>())],
        would_run: Vec::new(),
    };
    let preview = Preview {
        tasks: vec![
            task("pacman package cache", 1 << 30, WouldFree::Exactly(1 << 20)),
            task("systemd journal", 1 << 31, WouldFree::About(1 << 30)),
            task("Coredumps", 1 << 20, WouldFree::NotKnown),
        ],
        inactive: Vec::new(),
    };
    let shown = preview.to_string();
    assert!(
        shown.contains("now    1.0 GiB   would free 1.0 MiB"),
        "{shown}"
    );
    assert!(shown.contains("would free about 1.0 GiB"), "{shown}");
    assert!(shown.contains("would free not known"), "{shown}");
    assert!(shown.contains("preview: … 2 more lines"), "{shown}");
    assert_eq!(shown.matches("preview: line").count(), 30, "{shown}");
    assert!(
        shown.contains("Would free about 1.0 GiB in total (Coredumps not known)."),
        "{shown}"
    );
    let json = serde_json::to_value(&preview).unwrap();
    assert_eq!(json["tasks"][0]["would_free"], 1 << 20);
    assert_eq!(json["tasks"][0]["estimate"], false);
    assert_eq!(json["tasks"][1]["estimate"], true);
    assert!(json["tasks"][2]["would_free"].is_null());
}

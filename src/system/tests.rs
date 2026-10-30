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
        "echo '==> finished dry run: 3 candidates'",
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
    assert_eq!(
        pacman.previews[0].summary.as_deref(),
        Some("==> finished dry run: 3 candidates")
    );
    assert!(
        preview
            .to_string()
            .contains("preview: ==> finished dry run")
    );
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

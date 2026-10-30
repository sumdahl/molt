//! Smoke tests: the built binary starts, reports its version, rejects bad input, and shares
//! the startup behaviour of specs/004-cli.md.
//!
//! Every run clears the environment and points `HOME` and every `XDG_*` variable into a
//! temporary directory, so no smoke test can see the real home directory.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

struct Home {
    _dir: TempDir,
    path: PathBuf,
}

impl Home {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let path = fs::canonicalize(dir.path()).unwrap();
        Self { _dir: dir, path }
    }

    fn config(&self, text: &str) {
        let dir = self.path.join(".config/molt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.toml"), text).unwrap();
    }

    fn rules(&self, text: &str) {
        let dir = self.path.join(".config/molt/rules");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("test.toml"), text).unwrap();
    }

    /// Writes `len` bytes at `rel` (and its parents) and returns its path.
    fn file(&self, rel: &str, len: usize) -> PathBuf {
        let path = self.path.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![b'x'; len]).unwrap();
        path
    }

    /// Runs `molt scan --json` and parses its report.
    fn scan_json(&self) -> (Option<i32>, serde_json::Value) {
        let out = self.molt(&["scan", "--json"], &[]);
        let json = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
        (out.status.code(), json)
    }

    fn molt(&self, args: &[&str], extra: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_molt"));
        cmd.args(args)
            .env_clear()
            .env("PATH", "")
            .env("HOME", &self.path)
            .env("XDG_CONFIG_HOME", self.path.join(".config"))
            .env("XDG_CACHE_HOME", self.path.join(".cache"))
            .env("XDG_DATA_HOME", self.path.join(".local/share"))
            .env("XDG_STATE_HOME", self.path.join(".local/state"))
            .envs(extra.iter().copied());
        cmd.output().expect("failed to run molt binary")
    }
}

fn molt(args: &[&str]) -> Output {
    Home::new().molt(args, &[])
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn is_root() -> bool {
    fs::metadata(Home::new().path).unwrap().uid() == 0
}

/// Every path under `root` with its size and mtime.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (u64, i64)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            stack.extend(fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
        }
        out.insert(path, (meta.size(), meta.mtime()));
    }
    out
}

#[test]
fn version_matches_cargo_manifest() {
    let out = molt(&["--version"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("molt {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_exits_zero() {
    let out = molt(&["--help"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Usage: molt"));
}

#[test]
fn unknown_flag_is_rejected() {
    let out = molt(&["--definitely-not-a-flag"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn cl_01_global_flags_are_accepted() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    for args in [&["--dry-run"][..], &["--live"], &["--yes", "--json"], &[]] {
        let out = molt(args);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
    }
}

#[test]
fn cl_02_dry_run_with_live_is_a_usage_error() {
    assert_eq!(molt(&["--dry-run", "--live"]).status.code(), Some(2));
}

#[test]
fn cl_03_cl_08_a_fatal_error_exits_1_with_one_molt_line() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.config("colour = \"red\"");

    let out = home.molt(&[], &[]);

    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.starts_with("molt: "), "{err}");
    assert!(err.contains("config.toml"), "{err}");
}

#[test]
fn cl_04_a_relative_home_is_fatal() {
    let out = Command::new(env!("CARGO_BIN_EXE_molt"))
        .env_clear()
        .env("HOME", "not/absolute")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn cl_09_json_errors_are_also_written_to_stdout() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.config("colour = \"red\"");

    let out = home.molt(&["--json"], &[]);

    assert_eq!(out.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(json["error"].as_str().unwrap().contains("config.toml"));
}

#[test]
fn p_05_startup_writes_nothing() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.config("whitelist = [\"~/keep\"]");
    let before = snapshot(&home.path);

    for args in [&["--dry-run"][..], &["--live"], &[]] {
        assert!(home.molt(args, &[]).status.success());
    }

    assert_eq!(snapshot(&home.path), before);
}

// molt scan (specs/005-scan.md)

/// A user rule matching `~/junk/*`, with an owning-tool command.
const JUNK_RULE: &str = r#"
[[rule]]
id = "junk"
name = "Test junk"
category = "apps"
kind = "regenerable"
paths = ["~/junk/*"]
clean_with = ["junk-tool", "clean"]
"#;

/// `JUNK_RULE` without an owning-tool command, so `clean` deletes its files itself.
const PLAIN_JUNK_RULE: &str = r#"
[[rule]]
id = "junk"
name = "Test junk"
category = "apps"
kind = "regenerable"
paths = ["~/junk/*"]
"#;

const LOCKED_RULE: &str = r#"
[[rule]]
id = "locked"
name = "Test locked"
category = "apps"
kind = "regenerable"
paths = ["~/locked/*"]
"#;

fn on_disk(path: &Path) -> u64 {
    fs::symlink_metadata(path).unwrap().blocks() * 512
}

fn rule<'a>(report: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    report["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("no rule {id} in {report}"))
}

#[test]
fn sc_01_scan_changes_nothing_even_with_live() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(JUNK_RULE);
    home.file("junk/a", 64 * 1024);
    home.file(".cache/thumbnails/large/t.png", 4096);
    let before = snapshot(&home.path);

    for args in [
        &["scan"][..],
        &["scan", "--live"],
        &["scan", "--dry-run"],
        &[],
    ] {
        let out = home.molt(args, &[]);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        assert!(
            !stderr(&out).contains("--live"),
            "{args:?}: {}",
            stderr(&out)
        );
    }
    let with_config_dry_run = home.molt(&["scan"], &[("MOLT_DRY_RUN", "1")]);

    assert!(
        stderr(&with_config_dry_run).is_empty(),
        "{}",
        stderr(&with_config_dry_run)
    );
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn sc_02_sizes_come_from_the_dry_run_and_skips_are_not_counted() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(JUNK_RULE);
    home.config("whitelist = [\"~/junk/keep\"]");
    let a = home.file("junk/a", 64 * 1024);
    home.file("junk/keep", 64 * 1024);
    let thumb = home.file(".cache/thumbnails/large/t.png", 4096);
    let thumbnails = home.path.join(".cache/thumbnails");

    let (code, report) = home.scan_json();

    assert_eq!(code, Some(0));
    let junk = rule(&report, "junk");
    assert_eq!(junk["bytes"], on_disk(&a));
    let keep = junk["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["path"].as_str().unwrap().ends_with("keep"))
        .unwrap();
    assert_eq!(keep["outcome"], "skipped");
    assert_eq!(keep["reason"], "whitelisted");
    let expected = on_disk(&thumbnails) + on_disk(&thumbnails.join("large")) + on_disk(&thumb);
    assert_eq!(rule(&report, "thumbnails")["bytes"], expected);
    assert_eq!(report["total"], on_disk(&a) + expected);
}

#[test]
fn sc_03_sc_05_the_report_is_grouped_sorted_and_totalled() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(JUNK_RULE);
    home.file("junk/a", 256 * 1024);
    home.file(".cache/thumbnails/t.png", 4096);

    let out = home.molt(&["scan"], &[]);
    let text = String::from_utf8_lossy(&out.stdout);

    assert!(out.status.success(), "{}", stderr(&out));
    let apps = text.find("Apps").expect(&text);
    let system = text.find("System").expect(&text);
    assert!(apps < system, "largest category first:\n{text}");
    assert!(
        text.contains("Test junk") && text.contains("junk-tool clean"),
        "{text}"
    );
    assert!(
        !text.contains("npm download cache"),
        "inactive rules are not listed:\n{text}"
    );
    assert!(text.contains("inactive"), "{text}");
    assert!(
        text.contains("Total:") && text.contains("molt clean"),
        "{text}"
    );
}

#[test]
fn sc_06_the_json_report_has_the_stable_shape() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(JUNK_RULE);
    let a = home.file("junk/a", 4096);

    let (_, report) = home.scan_json();

    for key in ["total", "partial", "rules", "skipped", "inactive"] {
        assert!(report.get(key).is_some(), "missing {key}: {report}");
    }
    let junk = rule(&report, "junk");
    assert_eq!(junk["name"], "Test junk");
    assert_eq!(junk["category"], "apps");
    assert_eq!(junk["mode"], "permanent");
    assert_eq!(
        junk["clean_with"],
        serde_json::json!(["junk-tool", "clean"])
    );
    let item = &junk["items"][0];
    assert_eq!(item["path"], a.to_str().unwrap());
    assert_eq!(item["outcome"], "would-remove");
    assert!(
        report["inactive"]
            .as_array()
            .unwrap()
            .contains(&"npm-cache".into())
    );
}

#[test]
fn sc_07_symlinks_are_reported_and_unreadable_dirs_make_it_partial() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(&(JUNK_RULE.to_owned() + LOCKED_RULE));
    home.file("junk/a", 4096);
    symlink("/etc", home.path.join("junk/link")).unwrap();
    let locked = home.path.join("locked");
    home.file("locked/x", 4096);

    let (code, report) = home.scan_json();
    assert_eq!(code, Some(0));
    let skipped = report["skipped"].as_array().unwrap();
    assert!(skipped.iter().any(|s| s["path"].as_str().unwrap().ends_with("junk/link") && s["reason"] == "symlink"), "{report}");
    assert_eq!(report["partial"], false);

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let (code, report) = home.scan_json();
    let human = home.molt(&["scan"], &[]);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(code, Some(3));
    assert_eq!(report["partial"], true);
    assert_eq!(human.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&human.stdout).contains("Partial"));
}

#[test]
fn lf_06_scan_shows_locked_folders_and_changes_no_mode() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(JUNK_RULE);
    home.file("junk/a", 4096);
    // What makepkg leaves behind: a folder its owner can enter but not list.
    let pkg = home.path.join("junk/build/pkg");
    home.file("junk/build/pkg/x", 4096);
    fs::set_permissions(&pkg, fs::Permissions::from_mode(0o111)).unwrap();

    let (code, report) = home.scan_json();
    let human = home.molt(&["scan"], &[]);
    let mode = fs::symlink_metadata(&pkg).unwrap().mode() & 0o7777;
    fs::set_permissions(&pkg, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(code, Some(0));
    assert_eq!(mode, 0o111);
    assert_eq!(
        rule(&report, "junk")["locked"],
        serde_json::json!([pkg.to_str().unwrap()])
    );
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("(1 locked)"), "{text}");
    assert!(
        text.contains("molt clean") && text.contains("ask"),
        "{text}"
    );
}

#[test]
fn sc_08_cl_01_global_flags_work_after_the_command_name() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.file(".cache/thumbnails/t.png", 4096);

    let after = home.molt(&["scan", "--json"], &[]);
    let before = home.molt(&["--json", "scan"], &[]);

    assert!(after.status.success());
    assert_eq!(after.stdout, before.stdout);
    assert!(serde_json::from_slice::<serde_json::Value>(&after.stdout).is_ok());
}

#[test]
fn sc_09_no_command_runs_scan() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.file(".cache/thumbnails/t.png", 4096);

    assert_eq!(
        home.molt(&["--json"], &[]).stdout,
        home.molt(&["scan", "--json"], &[]).stdout
    );
}

// molt clean (specs/006-clean.md)

/// A fake tool in `<home>/../bin`: a shell script that records each run in `<name>.ran`.
fn fake_tool(home: &Home, name: &str, exit: i32) -> PathBuf {
    let bin = home.path.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tool = bin.join(name);
    let body = format!(
        "#!/bin/sh\n/bin/cp \"$MOLT_TEST_STDOUT\" \"{ran}\" 2>/dev/null || /bin/touch \"{ran}\"\nexit {exit}\n",
        ran = home.path.join(format!("{name}.ran")).display()
    );
    fs::write(&tool, body).unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

const TOOL_RULE: &str = r#"
[[rule]]
id = "tooled"
name = "Tool-cleaned cache"
category = "dev-tools"
kind = "regenerable"
paths = ["~/tooled"]
clean_with = ["fake-clean", "--all"]
requires = ["fake-clean"]
"#;

impl Home {
    fn clean(&self, args: &[&str]) -> Output {
        let mut all = vec!["clean"];
        all.extend_from_slice(args);
        self.molt(&all, &[])
    }

    /// Runs molt in a pseudo-terminal (via `script`), typing `input`. `None` if `script`
    /// isn't installed.
    fn molt_tty(&self, args: &str, input: &str) -> Option<Output> {
        self.molt_tty_with_path(args, input, "")
    }

    /// As `molt_tty`, with molt's `$PATH` set to `path`.
    fn molt_tty_with_path(&self, args: &str, input: &str, path: &str) -> Option<Output> {
        let script = Path::new("/usr/bin/script");
        if !script.exists() {
            return None;
        }
        let command = format!(
            "/usr/bin/env PATH={path} {} {args}",
            env!("CARGO_BIN_EXE_molt")
        );
        let mut child = Command::new(script)
            .args(["-qec", &command, "/dev/null"])
            .env_clear()
            .env("PATH", "")
            .env("HOME", &self.path)
            .env("XDG_CONFIG_HOME", self.path.join(".config"))
            .env("XDG_CACHE_HOME", self.path.join(".cache"))
            .env("XDG_DATA_HOME", self.path.join(".local/share"))
            .env("XDG_STATE_HOME", self.path.join(".local/state"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        Some(child.wait_with_output().unwrap())
    }
}

/// Spawns a just-copied binary. Another test thread's fork may briefly hold the copy's
/// write handle, and exec then fails with "Text file busy", so retry that.
fn spawn_fresh_copy(command: &mut Command) -> std::process::Child {
    for _ in 0..100 {
        match command.stdin(Stdio::piped()).spawn() {
            Err(e) if e.raw_os_error() == Some(rustix::io::Errno::TXTBSY.raw_os_error()) => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            spawned => return spawned.unwrap(),
        }
    }
    panic!("the copied binary stayed busy")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn cn_01_cn_11_yes_cleans_what_the_preview_shows() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(PLAIN_JUNK_RULE);
    home.config("whitelist = [\"~/junk/keep\"]");
    let a = home.file("junk/a", 64 * 1024);
    let keep = home.file("junk/keep", 4096);
    home.file(".cache/thumbnails/t.png", 4096);

    let out = home.clean(&["--yes"]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(!exists(&a) && !exists(&home.path.join(".cache/thumbnails")));
    assert!(exists(&keep));
    let text = stdout(&out);
    assert!(text.contains("Freed"), "{text}");
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[test]
fn cn_02_only_and_skip_narrow_the_run_and_unknown_ids_are_usage_errors() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(PLAIN_JUNK_RULE);
    let a = home.file("junk/a", 4096);
    let thumbs = home.file(".cache/thumbnails/t.png", 4096);

    assert_eq!(
        home.clean(&["--yes", "--only", "nope"]).status.code(),
        Some(2)
    );
    assert!(exists(&a) && exists(&thumbs));
    assert_eq!(
        home.clean(&["--yes", "--skip", "nope"]).status.code(),
        Some(2)
    );

    assert!(home.clean(&["--yes", "--skip", "junk"]).status.success());
    assert!(exists(&a) && !exists(&thumbs));
    assert!(home.clean(&["--yes", "--only", "junk"]).status.success());
    assert!(!exists(&a));
}

#[test]
fn cn_03_rules_whose_programs_are_running_are_left_out() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(
        &PLAIN_JUNK_RULE.replace("paths = [", "processes = [\"molt-busy-test\"]\npaths = ["),
    );
    let a = home.file("junk/a", 4096);
    let busy = home.path.join("molt-busy-test");
    // `sh` blocked on a builtin; a multicall `sleep` would exit under an unknown name.
    fs::copy("/bin/sh", &busy).unwrap();
    let mut program = spawn_fresh_copy(Command::new(&busy).args(["-c", "read line"]));
    // Until it has exec'd, /proc still shows the test binary.
    let comm = format!("/proc/{}/comm", program.id());
    while !fs::read_to_string(&comm).is_ok_and(|c| c.trim_end() == "molt-busy-test") {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let out = home.clean(&["--yes"]);
    program.kill().unwrap();
    program.wait().unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    assert!(exists(&a));
    assert!(stdout(&out).contains("molt-busy-test"), "{}", stdout(&out));
}

#[test]
fn cn_04_cn_08_the_preview_is_printed_before_the_first_change() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(TOOL_RULE);
    home.file("tooled/x", 4096);
    let bin = fake_tool(&home, "fake-clean", 0);
    let captured = home.path.join("stdout.txt");

    let status = Command::new(env!("CARGO_BIN_EXE_molt"))
        .args(["clean", "--yes"])
        .env_clear()
        .env("HOME", &home.path)
        .env("PATH", &bin)
        .env("MOLT_TEST_STDOUT", &captured)
        .stdout(fs::File::create(&captured).unwrap())
        .status()
        .unwrap();

    assert!(status.success());
    let seen = fs::read_to_string(home.path.join("fake-clean.ran")).unwrap();
    assert!(
        seen.contains("Tool-cleaned cache") && seen.contains("fake-clean --all"),
        "{seen}"
    );
    assert!(
        !seen.contains("Freed"),
        "the tool ran after the result:\n{seen}"
    );
    assert!(
        exists(&home.path.join("tooled/x")),
        "molt must not delete a tool's files"
    );
}

#[test]
fn cn_05_no_means_nothing_and_no_terminal_needs_yes() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.file(".cache/thumbnails/t.png", 4096);
    let before = snapshot(&home.path);

    let piped = home.clean(&[]);
    assert_eq!(piped.status.code(), Some(1));
    assert!(
        stdout(&piped).contains("Thumbnail cache"),
        "the preview is still shown"
    );
    assert!(stderr(&piped).contains("--yes"), "{}", stderr(&piped));
    assert_eq!(snapshot(&home.path), before);

    let Some(no) = home.molt_tty("clean", "n\n") else {
        return eprintln!("skipped the terminal half: script(1) isn't installed");
    };
    assert_eq!(no.status.code(), Some(0), "{}", stdout(&no));
    assert!(stdout(&no).contains("Nothing changed"), "{}", stdout(&no));
    assert_eq!(snapshot(&home.path), before);

    let yes = home.molt_tty("clean", "y\n").unwrap();
    assert_eq!(yes.status.code(), Some(0), "{}", stdout(&yes));
    assert!(!exists(&home.path.join(".cache/thumbnails")));
}

#[test]
fn cn_06_lf_05_locked_folders_need_their_own_consent() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(PLAIN_JUNK_RULE);
    let pkg = home.path.join("junk/build/pkg");
    home.file("junk/build/pkg/x", 4096);
    home.file("junk/build/src", 4096);
    fs::set_permissions(&pkg, fs::Permissions::from_mode(0o111)).unwrap();

    let yes_only = home.clean(&["--yes"]);
    let mode = fs::symlink_metadata(&pkg).unwrap().mode() & 0o7777;
    assert_eq!(yes_only.status.code(), Some(3), "kept folders are partial");
    assert_eq!(mode, 0o111, "--yes alone is not consent");
    assert!(!exists(&home.path.join("junk/build/src")));

    let consent = home.clean(&["--yes", "--fix-permissions"]);
    assert_eq!(consent.status.code(), Some(0), "{}", stderr(&consent));
    assert!(!exists(&home.path.join("junk/build")));
}

#[test]
fn cn_07_a_dry_run_shows_the_preview_and_changes_nothing() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(TOOL_RULE);
    home.file("tooled/x", 4096);
    home.file(".cache/thumbnails/t.png", 4096);
    let bin = fake_tool(&home, "fake-clean", 0);
    let before = snapshot(&home.path);
    let path = bin.to_str().unwrap();

    let flag = home.molt(&["clean", "--dry-run", "--yes"], &[("PATH", path)]);
    let env = home.molt(
        &["clean", "--yes"],
        &[("PATH", path), ("MOLT_DRY_RUN", "1")],
    );

    assert!(flag.status.success() && env.status.success());
    assert!(stdout(&flag).contains("Thumbnail cache"));
    assert!(
        stderr(&env).contains("MOLT_DRY_RUN") && stderr(&env).contains("--live"),
        "{}",
        stderr(&env)
    );
    assert!(
        !exists(&home.path.join("fake-clean.ran")),
        "a dry run never runs a tool"
    );
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn cn_08_a_failing_tool_is_reported_and_its_files_are_left_alone() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(TOOL_RULE);
    let x = home.file("tooled/x", 4096);
    let bin = fake_tool(&home, "fake-clean", 1);

    let out = home.molt(
        &["clean", "--yes", "--json"],
        &[("PATH", bin.to_str().unwrap())],
    );

    assert_eq!(out.status.code(), Some(3));
    assert!(exists(&x));
    let last = stdout(&out).lines().last().unwrap().to_owned();
    let result: serde_json::Value = serde_json::from_str(&last).unwrap();
    let rule = &result["result"]["rules"][0];
    assert_eq!(rule["id"], "tooled");
    assert_eq!(rule["ok"], false);
    assert_eq!(rule["command"]["exit"], 1);
}

#[test]
fn cn_09_anything_changed_since_the_preview_is_skipped() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(&(TOOL_RULE.to_owned() + PLAIN_JUNK_RULE));
    home.file("tooled/x", 4096);
    let a = home.file("junk/a", 4096);
    // The tool runs before deletions (CN-09) and replaces junk/a with a new file.
    let bin = home.path.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tool = bin.join("fake-clean");
    fs::write(
        &tool,
        format!(
            "#!/bin/sh\n/bin/mv \"{a}\" \"{a}.old\" && echo new > \"{a}\"\n",
            a = a.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();

    let out = home.molt(&["clean", "--yes"], &[("PATH", bin.to_str().unwrap())]);

    assert_eq!(out.status.code(), Some(3), "{}", stdout(&out));
    assert_eq!(
        fs::read_to_string(&a).unwrap(),
        "new\n",
        "the replacement must survive"
    );
}

#[test]
fn cn_10_permanent_and_trash_override_the_mode() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(PLAIN_JUNK_RULE);
    home.file("junk/a", 4096);

    assert_eq!(
        home.clean(&["--yes", "--trash", "--permanent"])
            .status
            .code(),
        Some(2)
    );
    let out = home.clean(&["--yes", "--trash"]);

    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("trash"), "{}", stdout(&out));
    assert!(exists(&home.path.join(".local/share/Trash/files/a")));
}

#[test]
fn cn_12_tool_commands_are_logged() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(TOOL_RULE);
    home.file("tooled/x", 4096);
    let bin = fake_tool(&home, "fake-clean", 0);

    assert!(
        home.molt(&["clean", "--yes"], &[("PATH", bin.to_str().unwrap())])
            .status
            .success()
    );

    let log = fs::read_to_string(home.path.join(".local/state/molt/operations.log")).unwrap();
    let entry: serde_json::Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
    assert_eq!(entry["command"], serde_json::json!(["fake-clean", "--all"]));
    assert_eq!(entry["exit"], 0);
}

/// Fake `sudo`, `paccache`, `journalctl` and `systemd-tmpfiles` in `<home>/bin`. Each
/// appends its name and arguments to `<home>/ran`; the fake `sudo` then runs the rest, so
/// no test ever runs the real `sudo` or touches the system.
fn fake_system_tools(home: &Home, failing: &str) -> PathBuf {
    let bin = home.path.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let ran = home.path.join("ran");
    for tool in ["sudo", "paccache", "journalctl", "systemd-tmpfiles"] {
        let run = if tool == "sudo" {
            "[ \"$1\" = -- ] || exit 99\nshift\nexec \"$@\""
        } else if tool == failing {
            "exit 1"
        } else {
            "echo done"
        };
        let body = format!(
            "#!/bin/sh\necho {tool} \"$@\" >> \"{}\"\n{run}\n",
            ran.display()
        );
        fs::write(bin.join(tool), body).unwrap();
        fs::set_permissions(bin.join(tool), fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

fn ran(home: &Home) -> String {
    fs::read_to_string(home.path.join("ran")).unwrap_or_default()
}

#[test]
fn sy_05_p_05_a_dry_run_runs_previews_only_and_never_sudo() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    let bin = fake_system_tools(&home, "");
    let bin = bin.to_str().unwrap();

    let out = home.molt(&["system", "--dry-run"], &[("PATH", bin)]);
    assert!(out.status.success(), "{}", stderr(&out));
    let ran = ran(&home);
    assert!(!ran.contains("sudo"), "{ran}");
    assert!(ran.contains("paccache -dk2\npaccache -duk0\n"), "{ran}");
    assert!(!ran.contains("-rk2"), "{ran}");
    // SY-03: the exact command lines are shown first.
    assert!(
        stdout(&out).contains(&format!("would run: {bin}/sudo -- {bin}/paccache -rk2")),
        "{}",
        stdout(&out)
    );
}

#[test]
fn sy_06_no_means_nothing_and_no_terminal_needs_yes() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    let bin = fake_system_tools(&home, "");
    let bin = bin.to_str().unwrap();

    let piped = home.molt(&["system"], &[("PATH", bin)]);
    assert_eq!(piped.status.code(), Some(1));
    assert!(
        stdout(&piped).contains("would run"),
        "the preview is still shown"
    );
    assert!(!ran(&home).contains("sudo"), "{}", ran(&home));

    let Some(no) = home.molt_tty_with_path("system", "n\n", bin) else {
        return eprintln!("skipped the terminal half: script(1) isn't installed");
    };
    assert_eq!(no.status.code(), Some(0), "{}", stdout(&no));
    assert!(stdout(&no).contains("Nothing changed"), "{}", stdout(&no));
    assert!(!ran(&home).contains("sudo"), "{}", ran(&home));
}

#[test]
fn sy_07_sy_08_sy_10_yes_runs_each_command_through_sudo_in_order() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    let bin = fake_system_tools(&home, "journalctl");
    let bin = bin.to_str().unwrap();

    let out = home.molt(&["system", "--yes", "--json"], &[("PATH", bin)]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    let ran = ran(&home);
    let sudo_lines: Vec<&str> = ran
        .lines()
        .filter(|line| line.starts_with("sudo"))
        .collect();
    assert_eq!(
        sudo_lines,
        [
            format!("sudo -- {bin}/paccache -rk2"),
            format!("sudo -- {bin}/paccache -ruk0"),
            format!("sudo -- {bin}/journalctl --vacuum-size=500M"),
            format!("sudo -- {bin}/systemd-tmpfiles --clean --prefix /var/lib/systemd/coredump"),
        ],
        "the failed journal task didn't stop the coredumps task"
    );
    let lines: Vec<serde_json::Value> = stdout(&out)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(lines[0]["preview"]["tasks"].is_array(), "{}", stdout(&out));
    assert_eq!(lines[1]["result"]["tasks"][1]["commands"][0]["exit"], 1);
    // SY-11
    let log = fs::read_to_string(home.path.join(".local/state/molt/operations.log")).unwrap();
    assert_eq!(log.lines().count(), 4, "{log}");
}

#[test]
fn sy_01_sy_02_unknown_ids_are_refused_and_relative_path_entries_ignored() {
    if is_root() {
        return eprintln!("skipped: molt refuses root (CL-07)");
    }
    let home = Home::new();
    fake_system_tools(&home, "");

    let typo = home.molt(&["system", "--only", "pacman", "--yes"], &[]);
    assert_eq!(typo.status.code(), Some(2));

    // `bin` is relative to molt's working directory, which is the temp home.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_molt"));
    let out = cmd
        .args(["system", "--yes", "--json"])
        .current_dir(&home.path)
        .env_clear()
        .env("PATH", "bin")
        .env("HOME", &home.path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(ran(&home).is_empty(), "{}", ran(&home));
    let preview: serde_json::Value =
        serde_json::from_str(stdout(&out).lines().next().unwrap()).unwrap();
    assert_eq!(
        preview["preview"]["inactive"],
        serde_json::json!(["pacman-cache", "journal", "coredumps"])
    );
}

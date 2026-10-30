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
        let dir = self.path.join(".config/onyx");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.toml"), text).unwrap();
    }

    fn rules(&self, text: &str) {
        let dir = self.path.join(".config/onyx/rules");
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

    /// Runs `onyx scan --json` and parses its report.
    fn scan_json(&self) -> (Option<i32>, serde_json::Value) {
        let out = self.onyx(&["scan", "--json"], &[]);
        let json = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
        (out.status.code(), json)
    }

    fn onyx(&self, args: &[&str], extra: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_onyx"));
        cmd.args(args)
            .env_clear()
            .env("PATH", "")
            .env("HOME", &self.path)
            .env("XDG_CONFIG_HOME", self.path.join(".config"))
            .env("XDG_CACHE_HOME", self.path.join(".cache"))
            .env("XDG_DATA_HOME", self.path.join(".local/share"))
            .env("XDG_STATE_HOME", self.path.join(".local/state"))
            .envs(extra.iter().copied());
        cmd.output().expect("failed to run onyx binary")
    }
}

fn onyx(args: &[&str]) -> Output {
    Home::new().onyx(args, &[])
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
    let out = onyx(&["--version"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("onyx {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_exits_zero() {
    let out = onyx(&["--help"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Usage: onyx"));
}

#[test]
fn unknown_flag_is_rejected() {
    let out = onyx(&["--definitely-not-a-flag"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn cl_01_global_flags_are_accepted() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    for args in [&["--dry-run"][..], &["--live"], &["--yes", "--json"], &[]] {
        let out = onyx(args);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
    }
}

#[test]
fn cl_02_dry_run_with_live_is_a_usage_error() {
    assert_eq!(onyx(&["--dry-run", "--live"]).status.code(), Some(2));
}

#[test]
fn cl_03_cl_08_a_fatal_error_exits_1_with_one_onyx_line() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.config("colour = \"red\"");

    let out = home.onyx(&[], &[]);

    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.starts_with("onyx: "), "{err}");
    assert!(err.contains("config.toml"), "{err}");
}

#[test]
fn cl_04_a_relative_home_is_fatal() {
    let out = Command::new(env!("CARGO_BIN_EXE_onyx"))
        .env_clear()
        .env("HOME", "not/absolute")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn cl_09_json_errors_are_also_written_to_stdout() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.config("colour = \"red\"");

    let out = home.onyx(&["--json"], &[]);

    assert_eq!(out.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(json["error"].as_str().unwrap().contains("config.toml"));
}

#[test]
fn p_05_startup_writes_nothing() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.config("whitelist = [\"~/keep\"]");
    let before = snapshot(&home.path);

    for args in [&["--dry-run"][..], &["--live"], &[]] {
        assert!(home.onyx(args, &[]).status.success());
    }

    assert_eq!(snapshot(&home.path), before);
}

// onyx scan (specs/005-scan.md)

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
        return eprintln!("skipped: onyx refuses root (CL-07)");
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
        let out = home.onyx(args, &[]);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        assert!(
            !stderr(&out).contains("--live"),
            "{args:?}: {}",
            stderr(&out)
        );
    }
    let with_config_dry_run = home.onyx(&["scan"], &[("ONYX_DRY_RUN", "1")]);

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
        return eprintln!("skipped: onyx refuses root (CL-07)");
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
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(JUNK_RULE);
    home.file("junk/a", 256 * 1024);
    home.file(".cache/thumbnails/t.png", 4096);

    let out = home.onyx(&["scan"], &[]);
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
        text.contains("Total:") && text.contains("onyx clean"),
        "{text}"
    );
}

#[test]
fn sc_06_the_json_report_has_the_stable_shape() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
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
        return eprintln!("skipped: onyx refuses root (CL-07)");
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
    let human = home.onyx(&["scan"], &[]);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(code, Some(3));
    assert_eq!(report["partial"], true);
    assert_eq!(human.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&human.stdout).contains("Partial"));
}

#[test]
fn lf_06_scan_shows_locked_folders_and_changes_no_mode() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(JUNK_RULE);
    home.file("junk/a", 4096);
    // What makepkg leaves behind: a folder its owner can enter but not list.
    let pkg = home.path.join("junk/build/pkg");
    home.file("junk/build/pkg/x", 4096);
    fs::set_permissions(&pkg, fs::Permissions::from_mode(0o111)).unwrap();

    let (code, report) = home.scan_json();
    let human = home.onyx(&["scan"], &[]);
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
        text.contains("onyx clean") && text.contains("ask"),
        "{text}"
    );
}

#[test]
fn sc_08_cl_01_global_flags_work_after_the_command_name() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.file(".cache/thumbnails/t.png", 4096);

    let after = home.onyx(&["scan", "--json"], &[]);
    let before = home.onyx(&["--json", "scan"], &[]);

    assert!(after.status.success());
    assert_eq!(after.stdout, before.stdout);
    assert!(serde_json::from_slice::<serde_json::Value>(&after.stdout).is_ok());
}

#[test]
fn sc_09_no_command_runs_scan() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.file(".cache/thumbnails/t.png", 4096);

    assert_eq!(
        home.onyx(&["--json"], &[]).stdout,
        home.onyx(&["scan", "--json"], &[]).stdout
    );
}

// onyx clean (specs/006-clean.md)

/// A fake tool in `<home>/../bin`: a shell script that records each run in `<name>.ran`.
fn fake_tool(home: &Home, name: &str, exit: i32) -> PathBuf {
    let bin = home.path.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let tool = bin.join(name);
    let body = format!(
        "#!/bin/sh\n/bin/cp \"$ONYX_TEST_STDOUT\" \"{ran}\" 2>/dev/null || /bin/touch \"{ran}\"\nexit {exit}\n",
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
        self.onyx(&all, &[])
    }

    /// Runs onyx in a pseudo-terminal (via `script`), typing `input`. `None` if `script`
    /// isn't installed.
    fn onyx_tty(&self, args: &str, input: &str) -> Option<Output> {
        self.onyx_tty_with_path(args, input, "")
    }

    /// As `onyx_tty`, with onyx's `$PATH` set to `path`.
    fn onyx_tty_with_path(&self, args: &str, input: &str, path: &str) -> Option<Output> {
        let script = Path::new("/usr/bin/script");
        if !script.exists() {
            return None;
        }
        let command = format!(
            "/usr/bin/env PATH={path} {} {args}",
            env!("CARGO_BIN_EXE_onyx")
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
        return eprintln!("skipped: onyx refuses root (CL-07)");
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
        return eprintln!("skipped: onyx refuses root (CL-07)");
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
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(
        &PLAIN_JUNK_RULE.replace("paths = [", "processes = [\"onyx-busy-test\"]\npaths = ["),
    );
    let a = home.file("junk/a", 4096);
    let busy = home.path.join("onyx-busy-test");
    // `sh` blocked on a builtin; a multicall `sleep` would exit under an unknown name.
    fs::copy("/bin/sh", &busy).unwrap();
    let mut program = spawn_fresh_copy(Command::new(&busy).args(["-c", "read line"]));
    // Until it has exec'd, /proc still shows the test binary.
    let comm = format!("/proc/{}/comm", program.id());
    while !fs::read_to_string(&comm).is_ok_and(|c| c.trim_end() == "onyx-busy-test") {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let out = home.clean(&["--yes"]);
    program.kill().unwrap();
    program.wait().unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    assert!(exists(&a));
    assert!(stdout(&out).contains("onyx-busy-test"), "{}", stdout(&out));
}

#[test]
fn cn_04_cn_08_the_preview_is_printed_before_the_first_change() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(TOOL_RULE);
    home.file("tooled/x", 4096);
    let bin = fake_tool(&home, "fake-clean", 0);
    let captured = home.path.join("stdout.txt");

    let status = Command::new(env!("CARGO_BIN_EXE_onyx"))
        .args(["clean", "--yes"])
        .env_clear()
        .env("HOME", &home.path)
        .env("PATH", &bin)
        .env("ONYX_TEST_STDOUT", &captured)
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
        "onyx must not delete a tool's files"
    );
}

#[test]
fn cn_05_no_means_nothing_and_no_terminal_needs_yes() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
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

    let Some(no) = home.onyx_tty("clean", "n\n") else {
        return eprintln!("skipped the terminal half: script(1) isn't installed");
    };
    assert_eq!(no.status.code(), Some(0), "{}", stdout(&no));
    assert!(stdout(&no).contains("Nothing changed"), "{}", stdout(&no));
    assert_eq!(snapshot(&home.path), before);

    let yes = home.onyx_tty("clean", "y\n").unwrap();
    assert_eq!(yes.status.code(), Some(0), "{}", stdout(&yes));
    assert!(!exists(&home.path.join(".cache/thumbnails")));
}

#[test]
fn cn_06_lf_05_locked_folders_need_their_own_consent() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
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
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(TOOL_RULE);
    home.file("tooled/x", 4096);
    home.file(".cache/thumbnails/t.png", 4096);
    let bin = fake_tool(&home, "fake-clean", 0);
    let before = snapshot(&home.path);
    let path = bin.to_str().unwrap();

    let flag = home.onyx(&["clean", "--dry-run", "--yes"], &[("PATH", path)]);
    let env = home.onyx(
        &["clean", "--yes"],
        &[("PATH", path), ("ONYX_DRY_RUN", "1")],
    );

    assert!(flag.status.success() && env.status.success());
    assert!(stdout(&flag).contains("Thumbnail cache"));
    assert!(
        stderr(&env).contains("ONYX_DRY_RUN") && stderr(&env).contains("--live"),
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
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(TOOL_RULE);
    let x = home.file("tooled/x", 4096);
    let bin = fake_tool(&home, "fake-clean", 1);

    let out = home.onyx(
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
        return eprintln!("skipped: onyx refuses root (CL-07)");
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

    let out = home.onyx(&["clean", "--yes"], &[("PATH", bin.to_str().unwrap())]);

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
        return eprintln!("skipped: onyx refuses root (CL-07)");
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
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(TOOL_RULE);
    home.file("tooled/x", 4096);
    let bin = fake_tool(&home, "fake-clean", 0);

    assert!(
        home.onyx(&["clean", "--yes"], &[("PATH", bin.to_str().unwrap())])
            .status
            .success()
    );

    let log = fs::read_to_string(home.path.join(".local/state/onyx/operations.log")).unwrap();
    // FP-06: the confirmation comes first.
    let entry: serde_json::Value = serde_json::from_str(log.lines().nth(1).unwrap()).unwrap();
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
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let bin = fake_system_tools(&home, "");
    let bin = bin.to_str().unwrap();

    let out = home.onyx(&["system", "--dry-run"], &[("PATH", bin)]);
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
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let bin = fake_system_tools(&home, "");
    let bin = bin.to_str().unwrap();

    let piped = home.onyx(&["system"], &[("PATH", bin)]);
    assert_eq!(piped.status.code(), Some(1));
    assert!(
        stdout(&piped).contains("would run"),
        "the preview is still shown"
    );
    assert!(!ran(&home).contains("sudo"), "{}", ran(&home));

    let Some(no) = home.onyx_tty_with_path("system", "n\n", bin) else {
        return eprintln!("skipped the terminal half: script(1) isn't installed");
    };
    assert_eq!(no.status.code(), Some(0), "{}", stdout(&no));
    assert!(stdout(&no).contains("Nothing changed"), "{}", stdout(&no));
    assert!(!ran(&home).contains("sudo"), "{}", ran(&home));
}

#[test]
fn sy_07_sy_08_sy_10_yes_runs_each_command_through_sudo_in_order() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let bin = fake_system_tools(&home, "journalctl");
    let bin = bin.to_str().unwrap();

    let out = home.onyx(&["system", "--yes", "--json"], &[("PATH", bin)]);
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
    // SY-11, and FP-06's confirmation.
    let log = fs::read_to_string(home.path.join(".local/state/onyx/operations.log")).unwrap();
    assert_eq!(log.lines().count(), 5, "{log}");
}

#[test]
fn sy_01_sy_02_unknown_ids_are_refused_and_relative_path_entries_ignored() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    fake_system_tools(&home, "");

    let typo = home.onyx(&["system", "--only", "pacman", "--yes"], &[]);
    assert_eq!(typo.status.code(), Some(2));

    // `bin` is relative to onyx's working directory, which is the temp home.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_onyx"));
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

const PURGE_CONFIG: &str = "purge_paths = [\"~/projects\"]\n";

/// Makes `rel` a month old, so its project isn't recent (PG-04).
fn backdate(home: &Home, rel: &str) {
    let month = std::time::Duration::from_secs(30 * 24 * 3600);
    fs::File::open(home.path.join(rel))
        .unwrap()
        .set_modified(std::time::SystemTime::now() - month)
        .unwrap();
}

/// An old Cargo project with a build, a `target` with no `Cargo.toml`, and a Cargo
/// project changed just now.
fn purge_home() -> (Home, PathBuf, PathBuf, PathBuf) {
    let home = Home::new();
    home.config(PURGE_CONFIG);
    home.file("projects/app/Cargo.toml", 10);
    backdate(&home, "projects/app/Cargo.toml");
    let built = home.file("projects/app/target/debug/app", 64 * 1024);
    let loose = home.file("projects/loose/target/keep", 4096);
    home.file("projects/fresh/Cargo.toml", 10);
    let fresh = home.file("projects/fresh/target/keep", 4096);
    (home, built, loose, fresh)
}

#[test]
fn pg_01_without_purge_paths_there_is_nothing_to_do() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.file("projects/app/Cargo.toml", 10);
    home.file("projects/app/target/x", 4096);

    let out = home.onyx(&["purge", "--yes"], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("purge_paths"), "{}", stderr(&out));
    assert!(exists(&home.path.join("projects/app/target/x")));
}

#[test]
fn pg_07_p_05_dry_run_and_no_change_nothing() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, ..) = purge_home();
    let before = snapshot(&home.path);

    let dry = home.onyx(&["purge", "--dry-run"], &[]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert!(
        stdout(&dry).contains("Cargo target folders"),
        "{}",
        stdout(&dry)
    );
    assert!(
        stdout(&dry).contains("with `onyx purge`"),
        "{}",
        stdout(&dry)
    );
    assert!(
        stdout(&dry).contains("Left out 1 build folders: changed in the last 7 days"),
        "{}",
        stdout(&dry)
    );
    assert_eq!(snapshot(&home.path), before);

    let piped = home.onyx(&["purge"], &[]);
    assert_eq!(piped.status.code(), Some(1), "no terminal needs --yes");
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn pg_02_pg_04_pg_07_yes_removes_only_old_builds_next_to_their_marker() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, built, loose, fresh) = purge_home();

    let out = home.onyx(&["purge", "--yes"], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!exists(&home.path.join("projects/app/target")));
    assert!(exists(&home.path.join("projects/app/Cargo.toml")));
    assert!(!exists(&built));
    assert!(exists(&loose), "no Cargo.toml next to it");
    assert!(exists(&fresh), "changed in the last 7 days");
    assert!(stdout(&out).contains("Freed"), "{}", stdout(&out));
    // CN-12
    let log = fs::read_to_string(home.path.join(".local/state/onyx/operations.log")).unwrap();
    assert!(log.contains("projects/app/target"), "{log}");
}

#[test]
fn pg_08_kinds_are_picked_and_unknown_kinds_refused() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, built, ..) = purge_home();

    let typo = home.onyx(&["purge", "--only", "rust", "--yes"], &[]);
    assert_eq!(typo.status.code(), Some(2));
    let other = home.onyx(&["purge", "--only", "node", "--yes"], &[]);
    assert!(other.status.success(), "{}", stderr(&other));
    assert!(exists(&built));

    let trashed = home.onyx(&["purge", "--trash", "--yes"], &[]);
    assert!(trashed.status.success(), "{}", stderr(&trashed));
    assert!(!exists(&built));
    assert!(exists(
        &home.path.join(".local/share/Trash/files/target/debug/app")
    ));
}

/// An old installer of each type in Downloads or Desktop, plus what must stay: a folder
/// named like one, a symlink, a new file, and one in a subfolder.
fn installer_home() -> (Home, Vec<PathBuf>, Vec<PathBuf>) {
    let home = Home::new();
    let old = [
        "Downloads/a.deb",
        "Downloads/b.rpm",
        "Downloads/c-1-x86_64.pkg.tar.zst",
        "Desktop/D.AppImage",
        "Downloads/os.iso",
        "Downloads/driver.run",
    ];
    let old: Vec<PathBuf> = old
        .iter()
        .map(|rel| {
            let path = home.file(rel, 4096);
            backdate(&home, rel);
            path
        })
        .collect();
    let folder = home.file("Downloads/folder.iso/inside", 10);
    backdate(&home, "Downloads/folder.iso");
    let link = home.path.join("Downloads/link.deb");
    std::os::unix::fs::symlink(&old[0], &link).unwrap();
    let new = home.file("Downloads/new.iso", 4096);
    let nested = home.file("Downloads/sub/e.deb", 4096);
    backdate(&home, "Downloads/sub/e.deb");
    (home, old, vec![folder, link, new, nested])
}

#[test]
fn in_01_scan_and_clean_never_see_installers() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, old, _) = installer_home();

    let (_, report) = home.scan_json();
    assert!(!report.to_string().contains("installer-"), "{report}");
    let out = home.clean(&["--yes"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(old.iter().all(|path| exists(path)));
}

#[test]
fn in_05_p_05_dry_run_and_no_change_nothing() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, ..) = installer_home();
    let before = snapshot(&home.path);

    let dry = home.onyx(&["installer", "--dry-run"], &[]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    let text = stdout(&dry);
    assert!(text.contains("Installers"), "{text}");
    assert!(text.contains("(to trash)"), "{text}");
    assert!(text.contains("with `onyx installer`"), "{text}");
    assert!(
        text.contains("Left out 1 installers: changed in the last 14 days"),
        "{text}"
    );
    assert_eq!(snapshot(&home.path), before);

    let piped = home.onyx(&["installer"], &[]);
    assert_eq!(piped.status.code(), Some(1), "no terminal needs --yes");
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn in_02_in_03_in_04_yes_trashes_only_old_files() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, old, kept) = installer_home();

    let out = home.onyx(&["installer", "--yes"], &[]);
    assert!(out.status.success(), "{}\n{}", stdout(&out), stderr(&out));
    for path in &old {
        assert!(!exists(path), "{}", path.display());
        let name = path.file_name().unwrap();
        assert!(exists(
            &home.path.join(".local/share/Trash/files").join(name)
        ));
    }
    for path in &kept {
        assert!(exists(path), "{}", path.display());
    }
}

#[test]
fn in_01_in_04_rules_are_picked_and_permanent_deletes() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, old, _) = installer_home();

    let typo = home.onyx(&["installer", "--only", "deb", "--yes"], &[]);
    assert_eq!(typo.status.code(), Some(2));
    let out = home.onyx(
        &[
            "installer",
            "--only",
            "installer-iso",
            "--permanent",
            "--yes",
        ],
        &[],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!exists(&old[4]));
    assert!(!exists(&home.path.join(".local/share/Trash/files/os.iso")));
    assert!(exists(&old[0]), "only the ISO rule ran");
}

#[test]
fn an_06_an_09_p_05_analyze_lists_largest_first_and_changes_nothing() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.file("data/big/file", 256 * 1024);
    home.file("data/small", 4096);
    let data = home.path.join("data");
    let before = snapshot(&home.path);

    let out = home.onyx(&["analyze", data.to_str().unwrap(), "--depth", "2"], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].ends_with(data.to_str().unwrap()), "{text}");
    assert!(lines[1].ends_with("  big/"), "{text}");
    assert!(lines[2].ends_with("    file"), "{text}");
    assert!(lines[3].ends_with("  small"), "{text}");

    let dry = home.onyx(&["analyze", data.to_str().unwrap(), "--dry-run"], &[]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert!(!stderr(&dry).contains("dry run"), "{}", stderr(&dry));
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn an_08_analyze_json_has_the_stable_shape() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.file("data/a/b", 4096);
    let data = home.path.join("data");

    let out = home.onyx(&["analyze", data.to_str().unwrap(), "--json"], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["name"], data.to_str().unwrap());
    assert_eq!(json["kind"], "dir");
    assert_eq!(json["partial"], false);
    assert!(json["bytes"].as_u64().unwrap() >= 4096);
    let a = &json["children"][0];
    assert_eq!((&a["name"], &a["kind"]), (&"a".into(), &"dir".into()));
    assert_eq!(a["children"], serde_json::json!([]));
}

#[test]
fn an_03_an_05_partial_exits_3_and_a_bad_path_exits_1() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let file = home.file("data/locked/x", 10);
    let locked = file.parent().unwrap();
    fs::set_permissions(locked, fs::Permissions::from_mode(0o000)).unwrap();
    let data = home.path.join("data");

    let partial = home.onyx(&["analyze", data.to_str().unwrap()], &[]);
    fs::set_permissions(locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(partial.status.code(), Some(3), "{}", stderr(&partial));
    assert!(stdout(&partial).contains("+"), "{}", stdout(&partial));

    let missing = home.onyx(&["analyze", "/nonexistent/onyx-test"], &[]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(stderr(&missing).contains("/nonexistent/onyx-test"));
    let not_dir = home.onyx(&["analyze", file.to_str().unwrap()], &[]);
    assert_eq!(not_dir.status.code(), Some(1));
    assert!(stderr(&not_dir).contains("not a directory"));
}

/// Runs `onyx analyze <dir> <args>` in a pseudo-terminal; see `onyx_pty`.
fn browse_tty(
    home: &Home,
    dir: &Path,
    args: &str,
    keys: &[&str],
    env: &[(&str, &str)],
) -> Option<(Option<i32>, String)> {
    onyx_pty(
        home,
        &format!("analyze {} {args}", dir.display()),
        keys,
        env,
    )
}

/// Runs `onyx <args>` in a pseudo-terminal under a timeout, typing each chunk of `keys`
/// after a pause, and gives the exit code and all it wrote. `None` where script(1) or
/// timeout(1) is missing.
fn onyx_pty(
    home: &Home,
    args: &str,
    keys: &[&str],
    env: &[(&str, &str)],
) -> Option<(Option<i32>, String)> {
    let (script, timeout) = (Path::new("/usr/bin/script"), Path::new("/usr/bin/timeout"));
    if !script.exists() || !timeout.exists() {
        return None;
    }
    let command = format!("{} {args}", env!("CARGO_BIN_EXE_onyx"));
    // A stuck screen or script(1) must fail the test, not hang it.
    let mut child = Command::new(timeout)
        .arg("20")
        .arg(script)
        .args(["-qec", &command, "/dev/null"])
        .env_clear()
        .env("HOME", &home.path)
        .env("TERM", "xterm")
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    let mut stdin = child.stdin.take().unwrap();
    for chunk in keys {
        // Keys typed before onyx switches to raw mode, or mid-measure, would be lost.
        std::thread::sleep(std::time::Duration::from_millis(400));
        stdin.write_all(chunk.as_bytes()).unwrap();
    }
    // Some script(1) versions wait for their stdin to close before exiting.
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    Some((
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    ))
}

/// AN-15: the last thing written puts the terminal back.
fn restored(screen: &str) -> bool {
    screen.trim_end().ends_with("\x1b[?25h\x1b[?1049l")
}

#[test]
fn an_10_an_13_an_15_the_browser_navigates_and_restores_the_terminal() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.file("data/big/inner-file", 256 * 1024);
    home.file("data/small", 4096);
    let data = home.path.join("data");
    let before = snapshot(&home.path);

    let Some((code, screen)) = browse_tty(&home, &data, "", &["l", "hq"], &[]) else {
        return eprintln!("skipped: script(1) or timeout(1) isn't installed");
    };

    assert_eq!(code, Some(0), "{screen:?}");
    assert!(
        screen.contains("\x1b[?1049h"),
        "the alternate screen: {screen:?}"
    );
    assert!(screen.contains("big/"), "{screen:?}");
    assert!(screen.contains("inner-file"), "opened big/: {screen:?}");
    assert!(restored(&screen), "{screen:?}");
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn an_20_an_21_an_22_a_marked_folder_goes_to_the_trash_after_the_preview_and_y() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let inside = home.file("data/big/inner-file", 256 * 1024);
    let small = home.file("data/small", 4096);
    let data = home.path.join("data");

    // Mark big/ (the largest, so selected first), trash, say yes, close the result, quit.
    let Some((code, screen)) = browse_tty(&home, &data, "", &[" ", "d", "y", "x", "q"], &[]) else {
        return eprintln!("skipped: script(1) or timeout(1) isn't installed");
    };

    assert_eq!(code, Some(0), "{screen:?}");
    assert!(screen.contains("1 marked"), "{screen:?}");
    assert!(
        screen.contains("Preview. Nothing has changed yet"),
        "{screen:?}"
    );
    assert!(screen.contains("would-trash"), "{screen:?}");
    assert!(screen.contains("Move 1 items"), "{screen:?}");
    assert!(screen.contains("trashed"), "{screen:?}");
    assert!(!exists(&inside));
    assert!(exists(
        &home.path.join(".local/share/Trash/files/big/inner-file")
    ));
    assert!(exists(&small), "only the marked item moved");
    assert!(restored(&screen), "{screen:?}");
    // AN-24
    let log = fs::read_to_string(home.path.join(".local/state/onyx/operations.log")).unwrap();
    assert!(log.contains("data/big"), "{log}");
}

#[test]
fn an_21_p_05_no_or_a_dry_run_changes_nothing() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.file("data/big/inner-file", 256 * 1024);
    let data = home.path.join("data");
    let before = snapshot(&home.path);

    let Some((code, screen)) = browse_tty(&home, &data, "", &["d", "n", "q"], &[]) else {
        return eprintln!("skipped: script(1) or timeout(1) isn't installed");
    };
    assert_eq!(code, Some(0), "{screen:?}");
    assert!(screen.contains("Move 1 items"), "{screen:?}");
    assert_eq!(snapshot(&home.path), before, "no");

    for (args, env) in [("--dry-run", &[][..]), ("", &[("ONYX_DRY_RUN", "1")][..])] {
        let (code, screen) = browse_tty(&home, &data, args, &["d", "y", "x", "q"], env).unwrap();
        assert_eq!(code, Some(0), "{screen:?}");
        assert!(screen.contains("would-trash"), "{screen:?}");
        assert!(screen.contains("Dry run, so nothing moved"), "{screen:?}");
        assert!(!screen.contains("Move 1 items"), "no question: {screen:?}");
        assert_eq!(snapshot(&home.path), before, "{args} {env:?}");
    }
}

#[test]
fn an_20_an_23_protected_items_are_refused_in_the_preview() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let key = home.file(".ssh/id_ed25519", 400 * 1024);
    let before = snapshot(&home.path);

    // ~/.ssh is the largest entry in the home, so it's selected.
    let Some((code, screen)) = browse_tty(&home, &home.path, "", &["d", "y", "q"], &[]) else {
        return eprintln!("skipped: script(1) or timeout(1) isn't installed");
    };

    assert_eq!(code, Some(0), "{screen:?}");
    assert!(screen.contains("protected"), "{screen:?}");
    assert!(
        screen.contains("Nothing here can be moved to the trash"),
        "{screen:?}"
    );
    assert!(exists(&key));
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn st_01_st_06_st_08_status_reports_and_changes_nothing() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let before = snapshot(&home.path);

    for args in [&["status"][..], &["status", "--dry-run"][..]] {
        let out = home.onyx(args, &[]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let text = stdout(&out);
        for section in ["CPU", "Memory", "Disks", "Processes"] {
            assert!(
                text.lines().any(|line| line == section),
                "{section}: {text}"
            );
        }
        assert!(text.contains("processes,"), "{text}");
    }
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn st_07_status_json_has_the_stable_shape() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let out = Home::new().onyx(&["status", "--json"], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(json["cpu"]["count"].as_u64().unwrap() >= 1, "{json}");
    assert!(json["memory"]["total"].as_u64().unwrap() > 0, "{json}");
    let root = json["disks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|disk| disk["point"] == "/")
        .unwrap_or_else(|| panic!("no / in {json}"));
    let share = root["used_share"].as_f64().unwrap();
    assert!((0.0..=100.0).contains(&share), "{root}");
    assert!(json["processes"]["total"].as_u64().unwrap() > 0, "{json}");
    assert!(json["processes"]["zombies"].is_array(), "{json}");
    // ST-13
    assert!(json["health"]["score"].as_u64().unwrap() <= 100, "{json}");
    assert!(json["health"]["reasons"].is_array(), "{json}");
}

#[test]
fn st_15_st_16_st_17_st_19_watch_refreshes_pauses_and_restores_the_terminal() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let before = snapshot(&home.path);

    // Waits past one 1 s refresh, pauses, then quits.
    let keys = ["", "", "", "p", "q"];
    let Some((code, screen)) = onyx_pty(&home, "status --watch --interval 1", &keys, &[]) else {
        return eprintln!("skipped: script(1) or timeout(1) isn't installed");
    };
    assert_eq!(code, Some(0), "{screen}");
    assert!(screen.contains("\nMemory"), "{screen}");
    assert!(screen.contains("every 1s"), "{screen}");
    assert!(screen.contains("paused"), "{screen}");
    assert!(restored(&screen), "{screen:?}");
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn st_18_watch_needs_a_terminal_and_no_json() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    for (args, why) in [
        (&["status", "--watch"][..], "needs a terminal"),
        (&["status", "--watch", "--json"][..], "--json"),
    ] {
        let out = home.onyx(args, &[]);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(stderr(&out).contains(why), "{args:?}: {}", stderr(&out));
    }
    for args in [
        &["status", "--interval", "5"][..],
        &["status", "--watch", "--interval", "0"][..],
    ] {
        assert_eq!(home.onyx(args, &[]).status.code(), Some(2), "{args:?}");
    }
}

/// A temp home with every UN-02 location filled for `foo`, plus things uninstall must
/// leave: `foo-bar`'s settings, a symlink at `~/.foo` and a whitelisted state folder.
/// Gives the home, foo's leftovers (settings first, then the cache) and what must stay.
fn uninstall_home() -> (Home, Vec<PathBuf>, PathBuf, Vec<PathBuf>) {
    let home = Home::new();
    let leftovers = [
        ".config/foo/settings.json",
        ".local/share/foo/data.db",
        ".local/share/applications/foo.desktop",
        ".config/autostart/foo.desktop",
    ]
    .map(|rel| home.file(rel, 4096))
    .to_vec();
    let cache = home.file(".cache/foo/blob", 4096);
    let other = home.file(".config/foo-bar/settings.json", 4096);
    let state = home.file(".local/state/foo/history", 4096);
    home.config("whitelist = [\"~/.local/state/foo\"]");
    let link = home.path.join(".foo");
    std::os::unix::fs::symlink(&other, &link).unwrap();
    (home, leftovers, cache, vec![other, state, link])
}

#[test]
fn un_01_un_03_bad_and_shared_names_are_usage_errors() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, ..) = uninstall_home();
    home.file(".local/share/Trash/files/old", 4096);
    let before = snapshot(&home.path);
    for name in [
        "../foo",
        "foo/..",
        ".foo",
        "f*",
        "applications",
        "Trash",
        "onyx",
    ] {
        let out = home.onyx(&["uninstall", name, "--yes"], &[]);
        assert_eq!(out.status.code(), Some(2), "{name}: {}", stdout(&out));
        assert!(stderr(&out).contains("nothing changed"), "{}", stderr(&out));
    }
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn un_06_un_07_p_05_dry_run_and_no_terminal_change_nothing() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, ..) = uninstall_home();
    let before = snapshot(&home.path);

    let dry = home.onyx(&["uninstall", "foo", "--dry-run"], &[]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    let text = stdout(&dry);
    assert!(text.contains("with `onyx uninstall foo`"), "{text}");
    assert!(text.contains("(to trash)"), "{text}");
    assert!(text.contains("foo itself isn't removed"), "{text}");
    assert_eq!(snapshot(&home.path), before);

    let piped = home.onyx(&["uninstall", "foo"], &[]);
    assert_eq!(piped.status.code(), Some(1), "no terminal needs --yes");
    assert_eq!(snapshot(&home.path), before);

    let none = home.onyx(&["uninstall", "nothing-here", "--yes"], &[]);
    assert!(none.status.success(), "{}", stderr(&none));
    assert!(
        stdout(&none).contains("Nothing of nothing-here in your home"),
        "{}",
        stdout(&none)
    );
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn un_02_un_05_yes_trashes_settings_and_deletes_the_cache() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, leftovers, cache, kept) = uninstall_home();

    let out = home.onyx(&["uninstall", "foo", "--yes"], &[]);
    assert!(out.status.success(), "{}\n{}", stdout(&out), stderr(&out));
    for path in leftovers.iter().chain([&cache]) {
        assert!(!exists(path), "{}", path.display());
    }
    for path in &kept {
        assert!(exists(path), "{}", path.display());
    }
    let trashed = fs::read_dir(home.path.join(".local/share/Trash/files"))
        .unwrap()
        .count();
    assert_eq!(
        trashed, 4,
        "settings, data and the two entries; not the cache"
    );
}

#[test]
fn un_05_permanent_puts_nothing_in_the_trash() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, leftovers, cache, _) = uninstall_home();

    let out = home.onyx(&["uninstall", "foo", "--permanent", "--yes"], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    for path in leftovers.iter().chain([&cache]) {
        assert!(!exists(path), "{}", path.display());
    }
    assert!(!exists(&home.path.join(".local/share/Trash/files")));
}

#[test]
fn un_04_a_running_app_is_left_alone() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let settings = home.file(".config/onyx-busy-app/settings", 4096);
    let busy = home.path.join("onyx-busy-app");
    fs::copy("/bin/sh", &busy).unwrap();
    let mut program = spawn_fresh_copy(Command::new(&busy).args(["-c", "read line"]));
    let comm = format!("/proc/{}/comm", program.id());
    while !fs::read_to_string(&comm).is_ok_and(|c| c.trim_end() == "onyx-busy-app") {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let out = home.onyx(&["uninstall", "onyx-busy-app", "--yes"], &[]);
    program.kill().unwrap();
    program.wait().unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    assert!(exists(&settings));
    assert!(
        stdout(&out).contains("quit onyx-busy-app first"),
        "{}",
        stdout(&out)
    );
}

/// Fake package tools and `sudo` in a temp dir of their own, so a snapshot of the home
/// stays exact. Each appends its name and arguments to `<dir>/ran` and then runs its
/// `body`; the fake `sudo` checks for `--` and runs the rest. No test runs a real one.
struct FakeTools {
    dir: TempDir,
}

impl FakeTools {
    fn new(tools: &[(&str, &str)]) -> Self {
        let dir = TempDir::new().unwrap();
        let ran = dir.path().join("ran");
        for (tool, body) in tools {
            let body = format!(
                "#!/bin/sh\necho {tool} \"$@\" >> \"{}\"\n{body}\n",
                ran.display()
            );
            fs::write(dir.path().join(tool), body).unwrap();
            fs::set_permissions(dir.path().join(tool), fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { dir }
    }

    fn bin(&self) -> &str {
        self.dir.path().to_str().unwrap()
    }

    fn ran(&self) -> String {
        fs::read_to_string(self.dir.path().join("ran")).unwrap_or_default()
    }
}

const FAKE_SUDO: (&str, &str) = ("sudo", "[ \"$1\" = -- ] || exit 99\nshift\nexec \"$@\"");

/// A pacman that has `foo` installed, owns `<bin>/bar` as `bar-pkg`, and removes with
/// `removal`'s exit code.
fn fake_pacman(removal: i32) -> String {
    format!(
        "case \"$1\" in\n\
         -Qq) [ \"$2\" = foo ] && echo foo || exit 1 ;;\n\
         -Qqo) case \"$2\" in */bar) echo bar-pkg ;; *) exit 1 ;; esac ;;\n\
         -Rs) echo \"$3-1.0\" ;;\n\
         -Rns) exit {removal} ;;\n\
         esac"
    )
}

fn package_home() -> (Home, Vec<PathBuf>) {
    let home = Home::new();
    let leftovers = vec![
        home.file(".config/foo/settings.json", 4096),
        home.file(".cache/foo/blob", 4096),
    ];
    (home, leftovers)
}

#[test]
fn un_08_un_10_p_05_a_dry_run_previews_the_package_and_never_runs_sudo() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, _) = package_home();
    let pacman = fake_pacman(0);
    let tools = FakeTools::new(&[FAKE_SUDO, ("pacman", &pacman)]);
    let bin = tools.bin();
    let before = snapshot(&home.path);

    let out = home.onyx(&["uninstall", "foo", "--dry-run"], &[("PATH", bin)]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("foo (pacman)"), "{text}");
    assert!(text.contains("    preview:\n      foo-1.0\n"), "{text}");
    assert!(
        text.contains(&format!("would run: {bin}/sudo -- {bin}/pacman -Rns foo")),
        "{text}"
    );
    assert!(!text.contains("isn't removed"), "{text}");
    let ran = tools.ran();
    assert!(
        ran.contains("pacman -Qq foo\npacman -Rs --print foo\n"),
        "{ran}"
    );
    assert!(!ran.contains("sudo"), "{ran}");
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn un_08_the_package_that_owns_the_program_on_path_is_found() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let pacman = fake_pacman(0);
    let tools = FakeTools::new(&[FAKE_SUDO, ("pacman", &pacman), ("bar", "exit 0")]);
    let bin = tools.bin();

    let out = home.onyx(&["uninstall", "bar", "--dry-run"], &[("PATH", bin)]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("bar-pkg (pacman)"), "{text}");
    assert!(
        text.contains(&format!("{bin}/pacman -Rns bar-pkg")),
        "{text}"
    );
    assert!(tools.ran().contains(&format!("pacman -Qqo {bin}/bar\n")));

    let none = home.onyx(&["uninstall", "nobody", "--dry-run"], &[("PATH", bin)]);
    assert!(
        stdout(&none).contains("nobody itself isn't removed"),
        "{}",
        stdout(&none)
    );
}

#[test]
fn un_11_un_13_yes_removes_the_package_then_the_leftovers() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, leftovers) = package_home();
    let pacman = fake_pacman(0);
    let tools = FakeTools::new(&[FAKE_SUDO, ("pacman", &pacman)]);
    let bin = tools.bin();

    let out = home.onyx(&["uninstall", "foo", "--yes", "--json"], &[("PATH", bin)]);
    assert!(out.status.success(), "{}\n{}", stdout(&out), stderr(&out));
    let sudo: Vec<String> = tools
        .ran()
        .lines()
        .filter(|line| line.starts_with("sudo"))
        .map(str::to_owned)
        .collect();
    // UN-11: no --noconfirm, so pacman asks for itself.
    assert_eq!(sudo, [format!("sudo -- {bin}/pacman -Rns foo")]);
    for path in &leftovers {
        assert!(!exists(path), "{}", path.display());
    }
    let lines: Vec<serde_json::Value> = stdout(&out)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        lines[0]["preview"]["package"]["name"],
        "foo",
        "{}",
        stdout(&out)
    );
    assert_eq!(lines[0]["preview"]["package"]["manager"], "pacman");
    assert_eq!(lines[1]["package"]["exit"], 0, "{}", stdout(&out));
    assert!(lines[2]["result"].is_object(), "{}", stdout(&out));
}

#[test]
fn un_12_a_failed_removal_keeps_the_leftovers() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, leftovers) = package_home();
    let pacman = fake_pacman(1);
    let tools = FakeTools::new(&[FAKE_SUDO, ("pacman", &pacman)]);

    let out = home.onyx(&["uninstall", "foo", "--yes"], &[("PATH", tools.bin())]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    for path in &leftovers {
        assert!(exists(path), "{}", path.display());
    }
    assert!(stdout(&out).contains("failed"), "{}", stdout(&out));
}

#[test]
fn un_15_without_sudo_only_the_leftovers_go() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, leftovers) = package_home();
    let pacman = fake_pacman(0);
    let tools = FakeTools::new(&[("pacman", &pacman)]);

    let out = home.onyx(&["uninstall", "foo", "--yes"], &[("PATH", tools.bin())]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("sudo isn't installed"),
        "{}",
        stdout(&out)
    );
    assert!(!tools.ran().contains("-Rns"), "{}", tools.ran());
    for path in &leftovers {
        assert!(!exists(path), "{}", path.display());
    }
}

#[test]
fn un_08_un_11_apt_and_dnf_use_their_own_commands() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let dpkg = "[ \"$1\" = -W ] && [ \"$3\" = foo ] && printf 'install ok installed' || exit 1";
    let rpm = "[ \"$1\" = -q ] && [ \"$2\" = foo ] && echo foo-1.0-1.x86_64 || exit 1";
    for (tools, removal) in [
        (
            vec![FAKE_SUDO, ("dpkg-query", dpkg), ("apt-get", "exit 0")],
            "apt-get remove foo",
        ),
        (
            vec![FAKE_SUDO, ("rpm", rpm), ("dnf", "exit 0")],
            "dnf remove foo",
        ),
    ] {
        let (home, _) = package_home();
        let tools = FakeTools::new(&tools);
        let bin = tools.bin();
        let out = home.onyx(&["uninstall", "foo", "--yes"], &[("PATH", bin)]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert!(
            tools.ran().contains(&format!("sudo -- {bin}/{removal}\n")),
            "{}",
            tools.ran()
        );
    }
}

/// A flatpak that has `org.example.App` for the user and `org.example.Sys` for the
/// system, and a snap that has `snapapp`.
const FAKE_FLATPAK: (&str, &str) = (
    "flatpak",
    "case \"$1 $2 $3\" in\n\
     'info --user org.example.App' | 'info --system org.example.Sys') echo Ref: app/$3 ;;\n\
     uninstall*) exit 0 ;;\n\
     *) exit 1 ;;\n\
     esac",
);
const FAKE_SNAP: (&str, &str) = (
    "snap",
    "case \"$1 $2\" in\n'list snapapp') echo snapapp 1.0 ;;\n'remove snapapp') exit 0 ;;\n*) exit 1 ;;\nesac",
);

#[test]
fn un_16_un_17_un_19_a_user_flatpak_goes_without_sudo_and_its_data_to_the_trash() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let data = home.file(".var/app/org.example.App/config/app.conf", 4096);
    let tools = FakeTools::new(&[FAKE_SUDO, FAKE_FLATPAK]);
    let bin = tools.bin();

    let dry = home.onyx(
        &["uninstall", "org.example.App", "--dry-run"],
        &[("PATH", bin)],
    );
    let text = stdout(&dry);
    assert!(text.contains("org.example.App (flatpak)"), "{text}");
    assert!(
        text.contains(&format!(
            "would run: {bin}/flatpak uninstall --user org.example.App\n"
        )),
        "{text}"
    );
    assert!(exists(&data));

    let out = home.onyx(&["uninstall", "org.example.App", "--yes"], &[("PATH", bin)]);
    assert!(out.status.success(), "{}", stderr(&out));
    let ran = tools.ran();
    assert!(
        ran.contains("flatpak uninstall --user org.example.App\n"),
        "{ran}"
    );
    assert!(!ran.contains("sudo"), "{ran}");
    assert!(!exists(&data));
    assert!(exists(
        &home.path.join(".local/share/Trash/files/org.example.App")
    ));
}

#[test]
fn un_16_un_17_a_system_flatpak_and_a_snap_go_through_sudo() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    for (name, removal) in [
        (
            "org.example.Sys",
            "flatpak uninstall --system org.example.Sys",
        ),
        ("snapapp", "snap remove snapapp"),
    ] {
        let home = Home::new();
        let tools = FakeTools::new(&[FAKE_SUDO, FAKE_FLATPAK, FAKE_SNAP]);
        let bin = tools.bin();
        let out = home.onyx(&["uninstall", name, "--yes"], &[("PATH", bin)]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert!(
            tools.ran().contains(&format!("sudo -- {bin}/{removal}\n")),
            "{}",
            tools.ran()
        );
    }
}

#[test]
fn un_19_un_20_un_21_appimages_icons_and_snap_data_are_leftovers() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let gone = [
        home.file("Applications/foo.AppImage", 4096),
        home.file(".local/share/icons/hicolor/256x256/apps/foo.png", 4096),
        home.file(".local/share/icons/hicolor/scalable/apps/foo.svg", 4096),
        home.file("snap/foo/common/data", 4096),
    ];
    let versioned = home.file("Applications/foo-1.2.AppImage", 4096);
    let link = home.path.join(".local/bin/foo.AppImage");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    symlink(&versioned, &link).unwrap();

    let out = home.onyx(&["uninstall", "foo", "--yes"], &[]);
    assert!(out.status.success(), "{}\n{}", stdout(&out), stderr(&out));
    for path in &gone {
        assert!(!exists(path), "{}", path.display());
    }
    assert!(exists(&versioned), "a versioned name is a partial match");
    assert!(fs::symlink_metadata(&link).is_ok(), "symlinks are skipped");
}

/// A pacman with orphans `a` and `b` that removes with `removal`'s exit code.
fn fake_orphan_pacman(removal: i32) -> String {
    format!("case \"$1\" in\n-Qdtq) printf 'a\\nb\\n' ;;\n-Rns) exit {removal} ;;\nesac")
}

#[test]
fn op_03_op_07_p_05_a_dry_run_lists_the_orphans_and_never_runs_sudo() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let pacman = fake_orphan_pacman(0);
    let tools = FakeTools::new(&[FAKE_SUDO, ("pacman", &pacman)]);
    let bin = tools.bin();
    let before = snapshot(&home.path);

    let out = home.onyx(&["optimize", "--dry-run"], &[("PATH", bin)]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("    a\n    b\n"), "{text}");
    assert!(
        text.contains(&format!("would run: {bin}/sudo -- {bin}/pacman -Rns a b\n")),
        "{text}"
    );
    assert_eq!(tools.ran(), "pacman -Qdtq\n");
    assert_eq!(snapshot(&home.path), before);

    let piped = home.onyx(&["optimize"], &[("PATH", bin)]);
    assert_eq!(piped.status.code(), Some(1), "no terminal needs --yes");
    assert!(!tools.ran().contains("sudo"));
}

#[test]
fn op_06_op_08_yes_removes_exactly_the_listed_packages() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let pacman = fake_orphan_pacman(0);
    let tools = FakeTools::new(&[
        FAKE_SUDO,
        ("pacman", &pacman),
        ("apt-get", "case \"$1\" in -s) echo 'Remv old [1]' ;; esac"),
    ]);
    let bin = tools.bin();

    let out = home.onyx(&["optimize", "--yes", "--json"], &[("PATH", bin)]);
    assert!(out.status.success(), "{}", stderr(&out));
    let ran = tools.ran();
    let sudo: Vec<&str> = ran.lines().filter(|l| l.starts_with("sudo")).collect();
    assert_eq!(
        sudo,
        [
            format!("sudo -- {bin}/pacman -Rns a b"),
            format!("sudo -- {bin}/apt-get autoremove"),
        ]
    );
    let lines: Vec<serde_json::Value> = stdout(&out)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        lines[0]["preview"]["tasks"][0]["found"]["packages"][1],
        "b",
        "{}",
        stdout(&out)
    );
    assert_eq!(
        lines[1]["result"]["commands"][0]["exit"],
        0,
        "{}",
        stdout(&out)
    );
}

#[test]
fn op_04_op_05_unsafe_names_and_empty_lists_run_nothing() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    for (body, says) in [
        ("[ \"$1\" = -Qdtq ] && printf 'a\\n-Rdd\\n'", "left out"),
        ("[ \"$1\" = -Qdtq ] && exit 1", "nothing to remove"),
    ] {
        let home = Home::new();
        let tools = FakeTools::new(&[FAKE_SUDO, ("pacman", body)]);
        let out = home.onyx(&["optimize", "--yes"], &[("PATH", tools.bin())]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert!(stdout(&out).contains(says), "{}", stdout(&out));
        assert!(!tools.ran().contains("sudo"), "{}", tools.ran());
    }
}

#[test]
fn op_01_op_02_op_08_tasks_are_picked_and_a_failure_doesnt_stop_the_next() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let pacman = fake_orphan_pacman(1);
    let dnf = "case \"$1\" in repoquery) printf 'x \\n' ;; esac";
    let tools = FakeTools::new(&[FAKE_SUDO, ("pacman", &pacman), ("dnf", dnf)]);
    let bin = tools.bin();

    let typo = home.onyx(
        &["optimize", "--only", "orphans", "--yes"],
        &[("PATH", bin)],
    );
    assert_eq!(typo.status.code(), Some(2));

    let out = home.onyx(&["optimize", "--yes"], &[("PATH", bin)]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert!(
        tools
            .ran()
            .contains(&format!("sudo -- {bin}/dnf remove x\n")),
        "{}",
        tools.ran()
    );

    let only = home.onyx(
        &["optimize", "--only", "dnf-autoremove", "--dry-run"],
        &[("PATH", bin)],
    );
    assert!(!stdout(&only).contains("pacman -Rns"), "{}", stdout(&only));

    // OP-02: no sudo, no tasks.
    let no_sudo = FakeTools::new(&[("pacman", &pacman)]);
    let out = home.onyx(&["optimize", "--yes"], &[("PATH", no_sudo.bin())]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("Inactive"), "{}", stdout(&out));
    assert!(!no_sudo.ran().contains("-Rns"));
}

const FAKE_RESOLVECTL: (&str, &str) = (
    "resolvectl",
    "[ \"$1\" = statistics ] && printf 'Cache\\n   Current Cache Size:  7\\n'; exit 0",
);

#[test]
fn op_09_op_10_p_05_a_dry_run_runs_only_the_dns_statistics() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let tools = FakeTools::new(&[
        FAKE_SUDO,
        ("fstrim", "exit 0"),
        ("fc-cache", "exit 0"),
        FAKE_RESOLVECTL,
    ]);
    let bin = tools.bin();

    let out = home.onyx(&["optimize", "--dry-run"], &[("PATH", bin)]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains(&format!(
            "would run: {bin}/sudo -- {bin}/fstrim --all --verbose\n"
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!("would run: {bin}/fc-cache\n")),
        "{text}"
    );
    assert!(text.contains("Current Cache Size: 7"), "{text}");
    assert!(
        text.contains(&format!(
            "would run: {bin}/sudo -- {bin}/resolvectl flush-caches\n"
        )),
        "{text}"
    );
    assert_eq!(tools.ran(), "resolvectl statistics\n");
}

#[test]
fn op_11_op_14_yes_runs_each_in_order_and_font_cache_without_sudo() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let tools = FakeTools::new(&[
        FAKE_SUDO,
        ("fstrim", "exit 1"),
        ("fc-cache", "exit 0"),
        FAKE_RESOLVECTL,
    ]);
    let bin = tools.bin();

    let out = home.onyx(&["optimize", "--yes"], &[("PATH", bin)]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "fstrim failed: {}",
        stderr(&out)
    );
    let ran = tools.ran();
    let live: Vec<&str> = ran
        .lines()
        .filter(|line| *line != "resolvectl statistics")
        .collect();
    assert_eq!(
        live,
        [
            format!("sudo -- {bin}/fstrim --all --verbose").as_str(),
            "fstrim --all --verbose",
            "fc-cache",
            &format!("sudo -- {bin}/resolvectl flush-caches"),
            "resolvectl flush-caches",
        ]
    );
}

#[test]
fn op_02_op_11_op_12_without_sudo_only_font_cache_and_no_resolved_no_dns() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let tools = FakeTools::new(&[
        ("fstrim", "exit 0"),
        ("fc-cache", "exit 0"),
        FAKE_RESOLVECTL,
    ]);
    let out = home.onyx(&["optimize", "--yes"], &[("PATH", tools.bin())]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(tools.ran(), "fc-cache\n");
    assert!(stdout(&out).contains("Inactive"), "{}", stdout(&out));

    let tools = FakeTools::new(&[FAKE_SUDO, ("resolvectl", "exit 1")]);
    let out = home.onyx(&["optimize", "--yes"], &[("PATH", tools.bin())]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("left out"), "{}", stdout(&out));
    assert!(!tools.ran().contains("flush-caches"), "{}", tools.ran());
}

const FAKE_SYSTEMCTL: (&str, &str) = (
    "systemctl",
    "case \"$*\" in\n\
     '--failed --plain --no-legend') echo 'bad.service loaded failed failed Bad' ;;\n\
     '--user --failed --plain --no-legend') echo 'mine.service loaded failed failed Mine' ;;\n\
     *) exit 1 ;;\n\
     esac",
);
const FAKE_SMARTCTL: (&str, &str) = (
    "smartctl",
    "echo 'SMART overall-health self-assessment test result: PASSED'",
);

#[test]
fn op_15_op_16_op_17_checks_only_read_and_never_use_sudo() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let tools = FakeTools::new(&[FAKE_SUDO, FAKE_SYSTEMCTL, FAKE_SMARTCTL]);
    let bin = tools.bin();

    for args in [&["optimize", "--dry-run"][..], &["optimize"][..]] {
        // No terminal and no --yes: checks alone never ask (OP-15).
        let out = home.onyx(args, &[("PATH", bin)]);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        let text = stdout(&out);
        assert!(text.contains("    bad.service\n"), "{text}");
        assert!(text.contains("    mine.service (user)\n"), "{text}");
        assert!(text.contains("systemctl status <unit>"), "{text}");
    }
    let ran = tools.ran();
    assert!(!ran.contains("sudo"), "{ran}");
    for line in ran.lines().filter(|line| line.starts_with("smartctl")) {
        let disk = line
            .strip_prefix("smartctl -H /dev/")
            .unwrap_or_else(|| panic!("{line}"));
        assert!(disk.bytes().all(|b| b.is_ascii_alphanumeric()), "{line}");
    }

    let skipped = home.onyx(
        &["optimize", "--skip", "failed-units", "smart"],
        &[("PATH", bin)],
    );
    assert!(
        !stdout(&skipped).contains("bad.service"),
        "{}",
        stdout(&skipped)
    );
}

#[test]
fn op_20_json_lists_each_checks_lines() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let tools = FakeTools::new(&[FAKE_SYSTEMCTL]);
    let out = home.onyx(&["optimize", "--json"], &[("PATH", tools.bin())]);
    assert!(out.status.success(), "{}", stderr(&out));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let task = &json["preview"]["tasks"][0];
    assert_eq!(task["id"], "failed-units", "{json}");
    assert_eq!(task["found"]["report"][0], "bad.service", "{json}");
}

/// The banner's art, as BN-01 draws it.
const BANNER_ART: &str = " \\___/|_| |_|\\__, /_/\\_\\";

#[test]
fn bn_03_no_banner_when_piped_with_json_or_for_version() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    for args in [
        &["--help"][..],
        &["help"][..],
        &[][..],
        &["--json"][..],
        &["--version"][..],
    ] {
        let out = home.onyx(args, &[]);
        assert!(
            !stdout(&out).contains(BANNER_ART),
            "{args:?}: {}",
            stdout(&out)
        );
        assert!(!stderr(&out).contains(BANNER_ART), "{args:?}");
    }
}

#[test]
fn bn_02_bn_04_the_banner_tops_help_on_a_terminal_in_green() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let Some((code, screen)) = onyx_pty(&home, "--help", &[], &[]) else {
        return eprintln!("skipped: script(1) or timeout(1) isn't installed");
    };
    assert_eq!(code, Some(0), "{screen}");
    let art = screen
        .find(BANNER_ART)
        .unwrap_or_else(|| panic!("{screen}"));
    let usage = screen.find("Usage:").unwrap_or_else(|| panic!("{screen}"));
    assert!(art < usage, "{screen}");
    assert!(screen.contains("\x1b[32m"), "{screen:?}");

    let (_, plain) = onyx_pty(&home, "--help", &[], &[("NO_COLOR", "1")]).unwrap();
    assert!(plain.contains(BANNER_ART), "{plain}");
    assert!(!plain.contains("\x1b[32m"), "{plain:?}");

    let (_, scan) = onyx_pty(&home, "scan", &[], &[]).unwrap();
    assert!(!scan.contains(BANNER_ART), "a named subcommand: {scan}");
}

/// A temp home whose log has a real `onyx installer --yes` run, then two command
/// entries and a corrupt line appended. Gives the home and the log's path.
fn history_home() -> (Home, PathBuf) {
    let (home, ..) = installer_home();
    let out = home.onyx(&["installer", "--yes"], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    let log = home.path.join(".local/state/onyx/operations.log");
    let mut text = fs::read_to_string(&log).unwrap();
    text.push_str(
        "{\"time\":\"2099-01-01T00:00:00Z\",\"command\":[\"/usr/bin/sudo\",\"--\",\"/usr/bin/fstrim\",\"--all\"],\"exit\":1,\"bytes\":0}\n\
         this line is broken\n",
    );
    fs::write(&log, text).unwrap();
    (home, log)
}

#[test]
fn hi_01_hi_02_hi_05_hi_06_history_reads_newest_first_and_changes_nothing() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, _) = history_home();
    let before = snapshot(&home.path);

    for args in [&["history"][..], &["history", "--dry-run"][..]] {
        let out = home.onyx(args, &[("TZ", "UTC")]);
        assert_eq!(
            out.status.code(),
            Some(3),
            "a bad line makes it partial: {}",
            stderr(&out)
        );
        let text = stdout(&out);
        let first = text.lines().next().unwrap();
        assert!(first.starts_with("2099-01-01 00:00  ran"), "{text}");
        assert!(first.ends_with("/usr/bin/fstrim --all  (exit 1)"), "{text}");
        assert_eq!(text.matches("  trashed  ").count(), 6, "{text}");
        assert!(text.contains("6 went to the trash"), "{text}");
        assert!(
            text.contains("1 line in the log couldn't be read"),
            "{text}"
        );
    }
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn hi_03_hi_04_limit_since_and_bad_dates() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, _) = history_home();

    let one = home.onyx(&["history", "--limit", "1"], &[]);
    let text = stdout(&one);
    assert_eq!(
        text.lines()
            .filter(|l| l.contains("  ran  ") || l.contains("  trashed  "))
            .count(),
        1,
        "{text}"
    );
    assert!(text.contains("6 older entries not shown"), "{text}");

    let future = home.onyx(&["history", "--since", "2098-12-31"], &[("TZ", "UTC")]);
    assert_eq!(
        stdout(&future).matches("  trashed  ").count(),
        0,
        "{}",
        stdout(&future)
    );
    assert!(stdout(&future).contains("fstrim"), "{}", stdout(&future));

    for bad in ["2026-13-01", "yesterday"] {
        assert_eq!(
            home.onyx(&["history", "--since", bad], &[]).status.code(),
            Some(2),
            "{bad}"
        );
    }
    assert_eq!(
        home.onyx(&["history", "--limit", "0"], &[]).status.code(),
        Some(2)
    );
}

#[test]
fn hi_06_hi_07_json_and_an_empty_history() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let (home, _) = history_home();
    let out = home.onyx(&["history", "--json", "--limit", "2"], &[]);
    assert_eq!(out.status.code(), Some(3));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["entries"].as_array().unwrap().len(), 2, "{json}");
    assert_eq!(json["entries"][0]["exit"], 1, "{json}");
    assert_eq!(json["entries"][1]["outcome"], "trashed", "{json}");
    assert_eq!(json["skipped_lines"], 1);
    assert_eq!(json["left_out"], 5);

    let empty = Home::new();
    let before = snapshot(&empty.path);
    let out = empty.onyx(&["history"], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("Nothing in the history yet"),
        "{}",
        stdout(&out)
    );
    assert_eq!(snapshot(&empty.path), before, "no state folder is created");
}

/// Completes `line` (words, the last one being completed) with onyx's bash script.
fn bash_complete(home: &Home, line: &str) -> Vec<String> {
    let script = home.path.join("onyx.bash");
    fs::write(&script, stdout(&home.onyx(&["completions", "bash"], &[]))).unwrap();
    let program = format!(
        "source '{}'\nCOMP_WORDS=({line})\nCOMP_CWORD=$((${{#COMP_WORDS[@]}} - 1))\n\
         [[ '{line}' == *' ' ]] && COMP_CWORD=${{#COMP_WORDS[@]}} && COMP_WORDS+=('')\n\
         _onyx\nprintf '%s\\n' \"${{COMPREPLY[@]}}\"",
        script.display()
    );
    let out = Command::new("/usr/bin/bash")
        .args(["--noprofile", "--norc", "-c", &program])
        .env_clear()
        .env("HOME", &home.path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    stdout(&out).lines().map(str::to_owned).collect()
}

#[test]
fn co_01_co_02_co_03_bash_completes_commands_flags_and_ids() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    if !Path::new("/usr/bin/bash").exists() {
        return eprintln!("skipped: bash isn't installed");
    }
    let home = Home::new();
    let out = home.onyx(&["completions", "bash"], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    let check = Command::new("/usr/bin/bash")
        .args(["-n", "-c", &stdout(&out)])
        .output()
        .unwrap();
    assert!(check.status.success(), "bash -n: {}", stderr(&check));

    assert_eq!(bash_complete(&home, "onyx cl"), ["clean"]);
    assert_eq!(
        bash_complete(&home, "onyx system --only "),
        ["pacman-cache", "journal", "coredumps"]
    );
    assert_eq!(bash_complete(&home, "onyx purge --skip c"), ["cargo"]);
    assert!(bash_complete(&home, "onyx clean --dr").contains(&"--dry-run".to_owned()));
    assert_eq!(bash_complete(&home, "onyx completions f"), ["fish"]);
    assert!(bash_complete(&home, "onyx optimize --only ").contains(&"dns-cache".to_owned()));
}

#[test]
fn co_01_unknown_shells_exit_2_and_a_broken_config_still_completes() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    assert_eq!(
        home.onyx(&["completions", "tcsh"], &[]).status.code(),
        Some(2)
    );
    home.config("this is not toml");
    let before = snapshot(&home.path);
    for shell in ["bash", "zsh", "fish"] {
        let out = home.onyx(&["completions", shell], &[]);
        assert!(out.status.success(), "{shell}: {}", stderr(&out));
        assert!(stdout(&out).contains("onyx"), "{shell}");
    }
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn co_04_zsh_and_fish_scripts_parse_where_installed() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    for (shell, check) in [("zsh", &["-n"][..]), ("fish", &["--no-execute"][..])] {
        let Some(path) = ["/usr/bin", "/bin"]
            .iter()
            .map(|dir| Path::new(dir).join(shell))
            .find(|path| path.exists())
        else {
            eprintln!("skipped: {shell} isn't installed");
            continue;
        };
        let script = home.path.join(format!("onyx.{shell}"));
        fs::write(&script, stdout(&home.onyx(&["completions", shell], &[]))).unwrap();
        let out = Command::new(path)
            .args(check)
            .arg(&script)
            .output()
            .unwrap();
        assert!(out.status.success(), "{shell}: {}", stderr(&out));
    }
}

#[test]
fn mn_01_a_piped_bare_onyx_is_still_a_scan() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let out = Home::new().onyx(&[], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("Total"), "{}", stdout(&out));
    assert!(!stdout(&out).contains("Enter run"), "{}", stdout(&out));
}

#[test]
fn mn_01_mn_02_the_menu_opens_on_a_terminal_and_q_restores_it() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let before = snapshot(&home.path);
    let Some((code, screen)) = onyx_pty(&home, "", &["j", "q"], &[]) else {
        return eprintln!("skipped: script(1) or timeout(1) isn't installed");
    };
    assert_eq!(code, Some(0), "{screen}");
    assert!(screen.contains("> Clean"), "j moved to Clean: {screen}");
    assert!(screen.contains("Enter run"), "{screen}");
    assert!(restored(&screen), "{screen:?}");
    assert_eq!(snapshot(&home.path), before, "MN-05");
}

#[test]
fn mn_03_mn_04_p_05_a_picked_command_runs_as_typed_and_comes_back() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(PLAIN_JUNK_RULE);
    let junk = home.file("junk/a", 4096);
    let before = snapshot(&home.path);

    // Clean, under --dry-run: the preview, the dry-run stop, Enter, then quit.
    let Some((code, screen)) = onyx_pty(&home, "--dry-run", &["j", "\r", "\r", "q"], &[]) else {
        return eprintln!("skipped: script(1) or timeout(1) isn't installed");
    };
    assert_eq!(code, Some(0), "{screen}");
    assert!(
        screen.contains("Preview. Nothing has changed yet"),
        "{screen}"
    );
    assert!(screen.contains("Test junk"), "{screen}");
    // MN-03: the menu's --dry-run reached the command, which stopped at its preview.
    assert!(!screen.contains("Clean all of the above?"), "{screen}");
    assert!(screen.contains("Press Enter to go back"), "{screen}");
    assert!(restored(&screen), "{screen:?}");
    assert!(exists(&junk));
    assert_eq!(snapshot(&home.path), before);

    // Uninstall with an empty name goes straight back.
    let keys = ["jjjj", "\r", "\r", "q"];
    let (code, screen) = onyx_pty(&home, "", &keys, &[]).unwrap();
    assert_eq!(code, Some(0), "{screen}");
    assert!(screen.contains("App to uninstall"), "{screen}");
    assert_eq!(snapshot(&home.path), before);
}

/// A signed fake release `v9.9.9` for `onyx update`, built with a test key, real `tar`,
/// `sha256sum` and `ssh-keygen`, and served by a fake `curl`. `pack` fills the release's
/// top folder before the default `onyx` (if it made none), packing and signing; `tamper` changes the release after. `None`
/// where those tools aren't installed.
struct FakeRelease {
    dir: TempDir,
}

const FAKE_TAG: &str = "v9.9.9";

impl FakeRelease {
    fn good() -> Option<Self> {
        Self::new(|_| {}, |_, _| {})
    }

    fn new(pack: impl Fn(&Path), tamper: impl Fn(&Path, &str)) -> Option<Self> {
        let tools = ["tar", "sha256sum", "ssh-keygen"];
        if !tools
            .iter()
            .all(|tool| Path::new("/usr/bin").join(tool).exists())
        {
            return None;
        }
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let run = |program: &str, args: &[&str]| {
            let out = Command::new(program)
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
            assert!(out.status.success(), "{program}: {}", stderr(&out));
            out
        };
        run(
            "ssh-keygen",
            &["-q", "-t", "ed25519", "-N", "", "-f", "key"],
        );
        let public = fs::read_to_string(root.join("key.pub")).unwrap();
        fs::write(
            root.join("signers"),
            format!("tester namespaces=\"onyx-release\" {public}"),
        )
        .unwrap();

        let top = format!("onyx-{FAKE_TAG}-x86_64-unknown-linux-musl");
        let release = root.join("release");
        fs::create_dir_all(release.join(&top)).unwrap();
        pack(&release.join(&top));
        let new = release.join(&top).join("onyx");
        if !exists(&new) {
            fs::write(&new, "#!/bin/sh\necho 'onyx 9.9.9'\n").unwrap();
            fs::set_permissions(&new, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(release.join(&top).join("README.md"), "readme").unwrap();
        let tarball = format!("{top}.tar.gz");
        let in_release = |args: &[&str]| {
            let out = Command::new(args[0])
                .args(&args[1..])
                .current_dir(&release)
                .output()
                .unwrap();
            assert!(out.status.success(), "{args:?}: {}", stderr(&out));
            out
        };
        in_release(&["tar", "czf", &tarball, &top]);
        let sum = in_release(&["sha256sum", &tarball]);
        fs::write(release.join(format!("{tarball}.sha256")), &sum.stdout).unwrap();
        let key = root.join("key");
        in_release(&[
            "ssh-keygen",
            "-q",
            "-Y",
            "sign",
            "-f",
            key.to_str().unwrap(),
            "-n",
            "onyx-release",
            &format!("{tarball}.sha256"),
        ]);
        tamper(&release, &tarball);

        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        let curl = format!(
            "#!/bin/sh\nout=''; url=''; head=0\n\
             while [ $# -gt 0 ]; do case \"$1\" in\n\
             --head) head=1 ;;\n\
             --output) out=\"$2\"; shift ;;\n\
             --write-out|--max-filesize|--proto|--proto-redir) shift ;;\n\
             -*) ;;\n\
             *) url=\"$1\" ;;\n\
             esac; shift; done\n\
             echo \"curl $url\" >> '{ran}'\n\
             if [ $head = 1 ]; then printf '%s' 'https://github.com/sumdahl/onyx/releases/tag/{FAKE_TAG}'; exit 0; fi\n\
             cp '{release}'/\"$(basename \"$url\")\" \"$out\"\n",
            ran = root.join("ran").display(),
            release = release.display(),
        );
        fs::write(bin.join("curl"), curl).unwrap();
        fs::set_permissions(bin.join("curl"), fs::Permissions::from_mode(0o755)).unwrap();
        Some(Self { dir })
    }

    fn ran(&self) -> String {
        fs::read_to_string(self.dir.path().join("ran")).unwrap_or_default()
    }
}

/// A copy of onyx installed in the temp home's `~/.local/bin`, run with the fake release
/// on `$PATH` and its test key allowed (SE-09).
fn installed_onyx(home: &Home, release: &FakeRelease, args: &[&str]) -> Output {
    let copy = home.path.join(".local/bin/onyx");
    if !copy.exists() {
        fs::create_dir_all(copy.parent().unwrap()).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_onyx"), &copy).unwrap();
    }
    let path = format!("{}:/usr/bin", release.dir.path().join("bin").display());
    let mut command = Command::new(&copy);
    command
        .args(args)
        .env_clear()
        .env("HOME", &home.path)
        .env("PATH", path)
        .env(
            "ONYX_TEST_ALLOWED_SIGNERS",
            release.dir.path().join("signers"),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn_fresh_copy(&mut command);
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}

#[test]
fn se_01_se_02_se_03_p_05_a_dry_run_checks_everything_and_changes_nothing() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let Some(release) = FakeRelease::good() else {
        return eprintln!("skipped: tar, sha256sum or ssh-keygen isn't installed");
    };
    let home = Home::new();
    installed_onyx(&home, &release, &["--version"]);
    let before = snapshot(&home.path);

    let out = installed_onyx(&home, &release, &["update", "--dry-run"]);
    assert!(out.status.success(), "{}\n{}", stdout(&out), stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("-> 9.9.9"), "{text}");
    assert_eq!(
        text.matches("  ok     ").count(),
        7,
        "every check passed: {text}"
    );
    assert!(text.contains("would replace"), "{text}");
    assert_eq!(snapshot(&home.path), before);
    assert!(
        release
            .ran()
            .lines()
            .all(|line| line.starts_with("curl https://github.com/sumdahl/onyx/releases/")),
        "{}",
        release.ran()
    );
}

#[test]
fn se_05_se_06_yes_swaps_in_the_new_onyx_and_trashes_the_old_one() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let Some(release) = FakeRelease::good() else {
        return eprintln!("skipped: tar, sha256sum or ssh-keygen isn't installed");
    };
    let home = Home::new();
    let out = installed_onyx(&home, &release, &["update", "--yes"]);
    assert!(out.status.success(), "{}\n{}", stdout(&out), stderr(&out));
    assert!(
        stdout(&out).contains("onyx is now 9.9.9"),
        "{}",
        stdout(&out)
    );
    let copy = home.path.join(".local/bin/onyx");
    assert_eq!(
        fs::read_to_string(&copy).unwrap(),
        "#!/bin/sh\necho 'onyx 9.9.9'\n"
    );
    let trashed: Vec<_> = fs::read_dir(home.path.join(".local/share/Trash/files"))
        .unwrap()
        .collect();
    assert_eq!(trashed.len(), 1, "the old onyx");
    let log = fs::read_to_string(home.path.join(".local/state/onyx/operations.log")).unwrap();
    assert!(log.contains("\"update\",\"--to\",\"9.9.9\""), "{log}");
    // SE-06: the confirmation (FP-06), the command and the old binary; the download
    // folder isn't logged.
    assert_eq!(log.lines().count(), 3, "{log}");
    assert!(log.contains("\"outcome\":\"trashed\""), "{log}");
}

#[test]
fn se_02_a_tampered_release_is_refused_and_nothing_is_replaced() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    type Pack = &'static dyn Fn(&Path);
    type Tamper = &'static dyn Fn(&Path, &str);
    let signed: Tamper = &|_, _| {};
    let cases: [(&str, Pack, Tamper); 5] = [
        ("signature", &|_| {}, &|release, tarball| {
            let sig = release.join(format!("{tarball}.sha256.sig"));
            fs::write(sig, "not a signature").unwrap();
        }),
        ("checksum", &|_| {}, &|release, tarball| {
            fs::write(release.join(tarball), "swapped").unwrap()
        }),
        (
            "version",
            &|top| fs::write(top.join("onyx"), "#!/bin/sh\necho 'onyx 1.0.0'\n").unwrap(),
            signed,
        ),
        (
            "symlink",
            &|top| std::os::unix::fs::symlink("/etc/passwd", top.join("link")).unwrap(),
            signed,
        ),
        (
            "binary is a folder",
            &|top| fs::create_dir(top.join("onyx")).unwrap(),
            signed,
        ),
    ];
    for (what, pack, tamper) in cases {
        let Some(release) = FakeRelease::new(pack, tamper) else {
            return eprintln!("skipped: tar, sha256sum or ssh-keygen isn't installed");
        };
        let home = Home::new();
        installed_onyx(&home, &release, &["--version"]);
        let before = snapshot(&home.path);
        let out = installed_onyx(&home, &release, &["update", "--yes"]);
        assert_eq!(out.status.code(), Some(1), "{what}: {}", stdout(&out));
        assert!(stdout(&out).contains("FAILED"), "{what}: {}", stdout(&out));
        assert_eq!(snapshot(&home.path), before, "{what}");
    }
}

#[test]
fn se_04_a_onyx_outside_your_home_is_left_to_its_installer() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let before = snapshot(&home.path);
    let out = home.onyx(&["update", "--yes"], &[]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("isn't a copy you own in your home"),
        "{}",
        stderr(&out)
    );
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn se_07_se_08_remove_trashes_onyx_and_keeps_the_log() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let Some(release) = FakeRelease::good() else {
        return eprintln!("skipped: tar, sha256sum or ssh-keygen isn't installed");
    };
    let home = Home::new();
    home.config("dry_run = false");
    home.file(".cache/onyx/x", 10);
    home.file(".local/state/onyx/operations.log", 10);
    home.file(".local/state/onyx/other", 10);
    home.file(".local/share/bash-completion/completions/onyx", 10);
    installed_onyx(&home, &release, &["--version"]);
    let before = snapshot(&home.path);

    let dry = installed_onyx(&home, &release, &["remove", "--dry-run"]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert_eq!(
        stdout(&dry).matches("would trash").count(),
        5,
        "{}",
        stdout(&dry)
    );
    assert!(
        stdout(&dry).contains("The operation log stays"),
        "{}",
        stdout(&dry)
    );
    assert_eq!(snapshot(&home.path), before);

    let out = installed_onyx(&home, &release, &["remove", "--yes"]);
    assert!(out.status.success(), "{}\n{}", stdout(&out), stderr(&out));
    for gone in [
        ".local/bin/onyx",
        ".config/onyx",
        ".cache/onyx",
        ".local/state/onyx/other",
        ".local/share/bash-completion/completions/onyx",
    ] {
        assert!(!exists(&home.path.join(gone)), "{gone}");
    }
    assert!(exists(&home.path.join(".local/state/onyx/operations.log")));
}

#[test]
fn se_07_remove_with_log_takes_the_operation_log_too() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let Some(release) = FakeRelease::good() else {
        return eprintln!("skipped: tar, sha256sum or ssh-keygen isn't installed");
    };
    let home = Home::new();
    home.file(".local/state/onyx/operations.log", 10);
    let out = installed_onyx(&home, &release, &["remove", "--with-log", "--yes"]);
    assert!(out.status.success(), "{}\n{}", stdout(&out), stderr(&out));
    assert!(!exists(&home.path.join(".local/state/onyx")));
    assert!(!exists(&home.path.join(".local/bin/onyx")));
}

#[test]
fn pk_02_a_cargo_installed_onyx_is_left_to_cargo() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let copy = home.path.join(".cargo/bin/onyx");
    fs::create_dir_all(copy.parent().unwrap()).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_onyx"), &copy).unwrap();
    let before = snapshot(&home.path);
    for args in [["update", "--yes"], ["remove", "--yes"]] {
        let mut command = Command::new(&copy);
        command
            .args(args)
            .env_clear()
            .env("HOME", &home.path)
            .env("PATH", "/usr/bin")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = spawn_fresh_copy(&mut command);
        drop(child.stdin.take());
        let out = child.wait_with_output().unwrap();
        let said = format!("{}{}", stdout(&out), stderr(&out));
        assert!(
            said.contains("cargo uninstall onyx-cli"),
            "{args:?}: {said}"
        );
        assert_eq!(snapshot(&home.path), before, "{args:?}");
    }
}

/// A fake `fprintd-verify` that records its call and exits `code`.
fn fake_fprintd(code: i32) -> FakeTools {
    FakeTools::new(&[("fprintd-verify", &format!("exit {code}"))])
}

fn confirmations(home: &Home) -> Vec<String> {
    fs::read_to_string(home.path.join(".local/state/onyx/operations.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|entry| entry["confirmed"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn fp_03_fp_06_a_matched_fingerprint_goes_ahead_without_typing() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.config("confirm = \"fingerprint\"");
    home.file(".cache/thumbnails/t.png", 4096);
    let tools = fake_fprintd(0);
    // No `y` typed: only the fingerprint can say yes.
    let Some(out) = home.onyx_tty_with_path("clean", "", tools.bin()) else {
        return eprintln!("skipped: script(1) isn't installed");
    };
    let text = stdout(&out);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("Touch the fingerprint sensor"), "{text}");
    assert!(!exists(&home.path.join(".cache/thumbnails")), "{text}");
    assert_eq!(tools.ran().lines().count(), 1, "{}", tools.ran());
    assert_eq!(confirmations(&home), ["fingerprint"]);
}

#[test]
fn fp_04_an_unconfirmed_or_missing_fingerprint_falls_back_to_typing() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let no_match = fake_fprintd(1);
    let missing = TempDir::new().unwrap();
    for (bin, said) in [
        (no_match.bin(), "wasn't confirmed"),
        (missing.path().to_str().unwrap(), "isn't installed"),
    ] {
        let home = Home::new();
        home.config("confirm = \"fingerprint\"");
        home.file(".cache/thumbnails/t.png", 4096);
        let before = snapshot(&home.path);
        let Some(no) = home.onyx_tty_with_path("clean", "\n", bin) else {
            return eprintln!("skipped: script(1) isn't installed");
        };
        let text = stdout(&no);
        assert!(text.contains(said) && text.contains("[y/N]"), "{text}");
        assert!(text.contains("Nothing changed"), "{text}");
        assert_eq!(snapshot(&home.path), before, "{said}");

        let yes = home.onyx_tty_with_path("clean", "y\n", bin).unwrap();
        assert_eq!(yes.status.code(), Some(0), "{}", stdout(&yes));
        assert!(!exists(&home.path.join(".cache/thumbnails")));
        assert_eq!(confirmations(&home), ["typed"], "{said}");
    }
}

#[test]
fn fp_02_yes_dry_runs_and_pipes_never_touch_the_sensor() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.config("confirm = \"fingerprint\"");
    home.file(".cache/thumbnails/t.png", 4096);
    let tools = fake_fprintd(0);
    let path = ("PATH", tools.bin());
    assert_eq!(home.onyx(&["clean"], &[path]).status.code(), Some(1));
    assert!(home.onyx(&["clean", "--dry-run"], &[path]).status.success());
    if let Some(dry) = home.onyx_tty_with_path("clean --dry-run", "", tools.bin()) {
        assert!(dry.status.success(), "{}", stdout(&dry));
    }
    assert!(home.onyx(&["clean", "--yes"], &[path]).status.success());
    assert_eq!(tools.ran(), "", "fprintd-verify never ran");
    assert_eq!(confirmations(&home), ["yes-flag"]);
}

#[test]
fn fp_01_a_bad_confirm_value_stops_onyx() {
    let home = Home::new();
    home.config("confirm = \"face\"");
    home.file(".cache/thumbnails/t.png", 4096);
    let before = snapshot(&home.path);
    let out = home.onyx(&["clean", "--yes"], &[]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(snapshot(&home.path), before);
}

#[test]
fn fp_05_ctrl_c_while_waiting_for_the_finger_changes_nothing() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.config("confirm = \"fingerprint\"");
    home.file(".cache/thumbnails/t.png", 4096);
    let before = snapshot(&home.path);
    // It would say yes after 3 s; Ctrl-C comes first.
    let tools = FakeTools::new(&[("fprintd-verify", "sleep 3\nexit 0")]);
    let path = format!("{}:/usr/bin", tools.bin());
    let Some((code, screen)) = onyx_pty(&home, "clean", &["\x03"], &[("PATH", &path)]) else {
        return eprintln!("skipped: script(1) or timeout isn't installed");
    };
    std::thread::sleep(std::time::Duration::from_secs(4));
    assert_ne!(code, Some(0), "{screen}");
    assert!(screen.contains("Touch the fingerprint sensor"), "{screen}");
    assert_eq!(snapshot(&home.path), before, "{screen}");
    assert!(confirmations(&home).is_empty());
}

#[test]
fn help_says_what_onyx_and_uninstall_do() {
    let help = stdout(&onyx(&["--help"]));
    // Its own about, not the crates.io description (PK-07).
    assert_eq!(
        help.lines().next(),
        Some("Reclaim disk space on Linux, safely")
    );
    assert!(
        help.contains("its package through its own manager"),
        "{help}"
    );
    assert!(!help.contains("left to your package manager"), "{help}");
}

#[test]
fn sy_04_preview_output_is_shown_once_inside_the_preview() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    let tools = FakeTools::new(&[
        FAKE_SUDO,
        (
            "paccache",
            "echo 'out line'\necho 'err line' >&2\necho '(disk space saved: 1.00 MiB)'",
        ),
    ]);
    let out = home.onyx(&["system", "--dry-run"], &[("PATH", tools.bin())]);
    assert!(out.status.success(), "{}", stderr(&out));
    let (text, err) = (stdout(&out), stderr(&out));
    assert!(
        !err.contains("out line") && !err.contains("err line"),
        "{err}"
    );
    assert_eq!(text.matches("preview: out line").count(), 2, "{text}");
    assert_eq!(text.matches("preview: err line").count(), 2, "{text}");
    assert!(text.contains("would free 2.0 MiB"), "{text}");
    assert!(text.contains("Would free 2.0 MiB in total."), "{text}");
}

#[test]
fn cn_08_cleanup_tools_run_in_home_not_where_onyx_was_started() {
    if is_root() {
        return eprintln!("skipped: onyx refuses root (CL-07)");
    }
    let home = Home::new();
    home.rules(TOOL_RULE);
    home.file("tooled/x", 4096);
    let bin = home.path.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let wrote = home.path.join("pwd");
    fs::write(
        bin.join("fake-clean"),
        format!("#!/bin/sh\n/bin/pwd > \"{}\"\n", wrote.display()),
    )
    .unwrap();
    fs::set_permissions(bin.join("fake-clean"), fs::Permissions::from_mode(0o755)).unwrap();
    // Started from elsewhere, as from inside a project.
    let elsewhere = TempDir::new().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_onyx"))
        .args(["clean", "--yes"])
        .current_dir(elsewhere.path())
        .env_clear()
        .env("HOME", &home.path)
        .env("PATH", &bin)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let pwd = fs::read_to_string(&wrote).unwrap();
    assert_eq!(
        Path::new(pwd.trim()).canonicalize().unwrap(),
        home.path.canonicalize().unwrap()
    );
}

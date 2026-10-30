//! Smoke tests: the built binary starts, reports its version, rejects bad input, and shares
//! the startup behaviour of specs/004-cli.md.
//!
//! Every run clears the environment and points `HOME` and every `XDG_*` variable into a
//! temporary directory, so no smoke test can see the real home directory.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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
fn sc_02_sc_07_an_unmeasurable_item_is_shown_as_failed() {
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
    fs::set_permissions(&pkg, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(code, Some(3));
    let build = rule(&report, "junk")["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["path"].as_str().unwrap().ends_with("junk/build"))
        .unwrap()
        .clone();
    assert_eq!(build["outcome"], "failed");
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("(1 failed)"), "{text}");
    assert!(text.contains("Partial"), "{text}");
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

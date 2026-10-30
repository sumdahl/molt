//! Tests for project discovery (specs/008-purge.md). Every project is a fake one in a
//! temp folder.

use super::*;

use std::process::Command;
use std::time::Duration;

use tempfile::TempDir;

/// A month from now: every project in the temp folder is old by then.
fn later() -> SystemTime {
    SystemTime::now() + Duration::from_secs(30 * 24 * 3600)
}

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"x").unwrap();
}

fn found(findings: &Findings) -> Vec<(String, PathBuf)> {
    let mut found: Vec<_> = findings
        .matches
        .iter()
        .map(|m| (m.rule.to_string(), m.candidate.path().to_owned()))
        .collect();
    found.sort();
    found
}

fn skipped(findings: &Findings, reason: SkipReason) -> Vec<PathBuf> {
    findings
        .skipped
        .iter()
        .filter(|skip| skip.reason == reason)
        .map(|skip| skip.path.clone())
        .collect()
}

fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .is_ok_and(|out| out.status.success())
}

fn git_path() -> Option<Vec<PathBuf>> {
    let git = ["/usr/bin", "/bin", "/usr/local/bin"]
        .iter()
        .map(PathBuf::from)
        .find(|dir| dir.join("git").exists())?;
    Some(vec![git])
}

#[test]
fn pg_kind_ids_are_valid_rule_ids() {
    for kind in ProjectKind::ALL {
        assert!(
            RuleId::try_from(kind.id().to_owned()).is_ok(),
            "{}",
            kind.id()
        );
    }
    assert_eq!(walk_id().to_string(), "purge-paths");
    assert!(RuleId::try_from("purge-paths".to_owned()).is_ok());
}

#[test]
fn pg_02_a_build_folder_needs_its_marker_next_to_it() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    touch(&root.join("app/Cargo.toml"));
    touch(&root.join("app/target/debug/app"));
    touch(&root.join("loose/target/x"));
    touch(&root.join("web/package.json"));
    touch(&root.join("web/node_modules/left-pad/index.js"));
    touch(&root.join("py/pyproject.toml"));
    touch(&root.join("py/.venv/pyvenv.cfg"));
    touch(&root.join("py2/requirements.txt"));
    touch(&root.join("py2/venv/not-a-venv"));
    touch(&root.join("site/package.json"));
    touch(&root.join("site/next.config.mjs"));
    touch(&root.join("site/.next/cache/x"));
    touch(&root.join("lib/build.gradle.kts"));
    touch(&root.join("lib/build/out.jar"));
    touch(&root.join("plain/build/report.pdf"));

    let findings = find(&[root.to_owned()], &[], later());
    assert_eq!(
        found(&findings),
        [
            ("cargo".to_owned(), root.join("app/target")),
            ("gradle".to_owned(), root.join("lib/build")),
            ("next".to_owned(), root.join("site/.next")),
            ("node".to_owned(), root.join("web/node_modules")),
            ("python-venv".to_owned(), root.join("py/.venv")),
        ]
    );
}

#[test]
fn pg_03_symlinks_hidden_folders_and_candidates_are_not_searched() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let root = dir.path().join("projects");
    touch(&outside.path().join("p/Cargo.toml"));
    touch(&outside.path().join("p/target/x"));
    fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(outside.path().join("p"), root.join("linked")).unwrap();
    touch(&root.join(".hidden/p/Cargo.toml"));
    touch(&root.join(".hidden/p/target/x"));
    touch(&root.join("app/Cargo.toml"));
    touch(&root.join("app/target/vendored/Cargo.toml"));
    touch(&root.join("app/target/vendored/target/x"));

    let findings = find(std::slice::from_ref(&root), &[], later());
    assert_eq!(
        found(&findings),
        [("cargo".to_owned(), root.join("app/target"))]
    );
}

#[test]
fn pg_03_a_symlinked_root_is_skipped() {
    let dir = TempDir::new().unwrap();
    touch(&dir.path().join("real/app/Cargo.toml"));
    touch(&dir.path().join("real/app/target/x"));
    std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();

    let findings = find(&[dir.path().join("link")], &[], later());
    assert!(findings.matches.is_empty());
    assert_eq!(
        skipped(&findings, SkipReason::Symlink),
        [dir.path().join("link")]
    );
}

#[test]
fn pg_04_recently_changed_projects_are_left_out() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    touch(&root.join("app/Cargo.toml"));
    touch(&root.join("app/target/x"));

    let now = find(&[root.to_owned()], &[], SystemTime::now());
    assert!(now.matches.is_empty());
    assert_eq!(
        skipped(&now, SkipReason::RecentlyChanged),
        [root.join("app/target")]
    );

    assert_eq!(find(&[root.to_owned()], &[], later()).matches.len(), 1);
}

#[test]
fn pg_04_changes_in_src_count_but_changes_in_the_build_folder_do_not() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    touch(&root.join("app/Cargo.toml"));
    touch(&root.join("app/src/main.rs"));
    touch(&root.join("app/target/x"));
    let week = Duration::from_secs(8 * 24 * 3600);
    let old = SystemTime::now() - week;
    for path in ["app/Cargo.toml", "app/src"] {
        fs::File::open(root.join(path))
            .unwrap()
            .set_modified(old)
            .unwrap();
    }

    // Only src/main.rs and target are new.
    let found_now = find(&[root.to_owned()], &[], SystemTime::now());
    assert!(found_now.matches.is_empty(), "src/main.rs is new");

    fs::File::open(root.join("app/src/main.rs"))
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert_eq!(
        find(&[root.to_owned()], &[], SystemTime::now())
            .matches
            .len(),
        1,
        "a new target alone doesn't make the project recent"
    );
}

#[test]
fn pg_05_git_tracked_build_folders_are_left_out() {
    let Some(search_path) = git_path() else {
        return eprintln!("skipped: git isn't installed");
    };
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    touch(&repo.join("tracked/Cargo.toml"));
    touch(&repo.join("tracked/target/checked-in.txt"));
    touch(&repo.join("ignored/Cargo.toml"));
    touch(&repo.join("ignored/target/x"));
    if !git(&repo, &["init", "-q"]) || !git(&repo, &["add", "tracked/target"]) {
        return eprintln!("skipped: git init failed");
    }

    let findings = find(std::slice::from_ref(&repo), &search_path, later());
    assert_eq!(
        found(&findings),
        [("cargo".to_owned(), repo.join("ignored/target"))]
    );
    assert_eq!(
        skipped(&findings, SkipReason::GitTracked),
        [repo.join("tracked/target")]
    );

    // P-02: without git, nothing in a repository is a candidate.
    let blind = find(&[repo], &[], later());
    assert!(blind.matches.is_empty());
    assert_eq!(skipped(&blind, SkipReason::GitTracked).len(), 2);
}

#[test]
fn pg_06_a_folder_two_kinds_claim_is_found_once() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    touch(&root.join("both/Cargo.toml"));
    touch(&root.join("both/pom.xml"));
    touch(&root.join("both/target/x"));

    let findings = find(&[root.to_owned()], &[], later());
    assert_eq!(found(&findings).len(), 1);
}

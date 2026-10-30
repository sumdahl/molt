//! Tests for the installer filter (specs/009-installer.md), on files in a temp folder.

use super::*;

use crate::rules::{Match, RuleId};
use crate::safe_delete::{Candidate, Mode};

use tempfile::TempDir;

fn found(path: &Path) -> Match {
    Match {
        rule: RuleId::try_from("installer-iso".to_owned()).unwrap(),
        candidate: Candidate::scan(path).unwrap(),
        mode: Mode::Trash,
    }
}

/// A month from now: every file in the temp folder is old by then.
fn later() -> SystemTime {
    SystemTime::now() + Duration::from_secs(30 * 24 * 3600)
}

#[test]
fn in_02_only_regular_files_are_kept() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("os.iso");
    let folder = dir.path().join("folder.iso");
    let link = dir.path().join("link.iso");
    fs::write(&file, b"x").unwrap();
    fs::create_dir(&folder).unwrap();
    std::os::unix::fs::symlink(&file, &link).unwrap();
    let mut findings = Findings {
        matches: vec![found(&file), found(&folder), found(&link)],
        ..Findings::default()
    };

    keep_old_files(&mut findings, later());

    let kept: Vec<&Path> = findings
        .matches
        .iter()
        .map(|m| m.candidate.path())
        .collect();
    assert_eq!(kept, [file.as_path()]);
    let skipped: Vec<(&Path, SkipReason)> = findings
        .skipped
        .iter()
        .map(|skip| (skip.path.as_path(), skip.reason))
        .collect();
    assert_eq!(
        skipped,
        [
            (folder.as_path(), SkipReason::NotAFile),
            (link.as_path(), SkipReason::Symlink)
        ]
    );
}

#[test]
fn in_03_files_from_the_last_14_days_are_left_out() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("os.iso");
    fs::write(&file, b"x").unwrap();

    let mut now = Findings {
        matches: vec![found(&file)],
        ..Findings::default()
    };
    keep_old_files(&mut now, SystemTime::now());
    assert!(now.matches.is_empty());
    assert_eq!(now.skipped[0].reason, SkipReason::RecentlyChanged);

    let mut old = Findings {
        matches: vec![found(&file)],
        ..Findings::default()
    };
    keep_old_files(&mut old, later());
    assert_eq!(old.matches.len(), 1);
}

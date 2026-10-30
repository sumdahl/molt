//! `onyx installer` (specs/009-installer.md): downloaded installers. Matching is the
//! rules engine's, and the preview, consent and deletion are `clean`'s (IN-05); this
//! only keeps the old, regular files.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::rules::{Findings, SkipReason, Skipped};

#[cfg(test)]
mod tests;

/// IN-03: a file this new may be one you're about to use.
const RECENT: Duration = Duration::from_secs(14 * 24 * 3600);

/// IN-02, IN-03: moves every match that isn't a regular file older than `RECENT` into
/// the skipped list, with its reason.
pub fn keep_old_files(findings: &mut Findings, now: SystemTime) {
    let cutoff = now.checked_sub(RECENT);
    for found in std::mem::take(&mut findings.matches) {
        match reason(found.candidate.path(), cutoff) {
            None => findings.matches.push(found),
            Some(reason) => findings.skipped.push(Skipped {
                path: found.candidate.path().to_owned(),
                rule: found.rule,
                reason,
            }),
        }
    }
}

/// Why `path` is left out, if it is. Unknown counts as recent (P-02).
fn reason(path: &Path, cutoff: Option<SystemTime>) -> Option<SkipReason> {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Some(SkipReason::RecentlyChanged);
    };
    if meta.file_type().is_symlink() {
        return Some(SkipReason::Symlink);
    }
    if !meta.is_file() {
        return Some(SkipReason::NotAFile);
    }
    match (meta.modified(), cutoff) {
        (Ok(modified), Some(cutoff)) if modified <= cutoff => None,
        _ => Some(SkipReason::RecentlyChanged),
    }
}

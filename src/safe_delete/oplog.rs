//! SD-15: the operation log (P-09), one JSON object per line.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use serde::Serialize;

use super::{Mode, Outcome};

pub(super) struct OpLog(File);

#[derive(Serialize)]
struct Entry<'a> {
    time: String,
    mode: Mode,
    path: &'a str,
    bytes: u64,
    outcome: &'static str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    errno: Option<i32>,
    /// LF-04: locked folders opened with `u+rwx` to remove this candidate.
    #[serde(skip_serializing_if = "is_zero")]
    permissions_fixed: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl OpLog {
    /// Opens the log for appending, creating it (mode 0600: it lists the user's paths).
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(path)
            .map(Self)
    }

    /// Appends an entry if `outcome` changed the filesystem.
    pub(super) fn record(
        &mut self,
        mode: Mode,
        path: &Path,
        outcome: &Outcome,
        permissions_fixed: u32,
    ) -> io::Result<()> {
        let (bytes, outcome, partial, errno) = match *outcome {
            Outcome::Removed { bytes, partial } => (bytes, "removed", partial, None),
            Outcome::Trashed { bytes } => (bytes, "trashed", false, None),
            Outcome::Failed { errno, bytes } if bytes > 0 => {
                (bytes, "failed", true, Some(errno.raw_os_error()))
            }
            _ => return Ok(()),
        };
        let entry = Entry {
            time: jiff::Timestamp::now().to_string(),
            mode,
            // SD-04 refuses non-UTF-8 paths before anything is changed.
            path: &path.to_string_lossy(),
            bytes,
            outcome,
            partial,
            errno,
            permissions_fixed,
        };
        let mut line = serde_json::to_string(&entry)?;
        line.push('\n');
        // One write per line, so an O_APPEND log never interleaves half entries.
        self.0.write_all(line.as_bytes())
    }
}

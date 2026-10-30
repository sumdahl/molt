//! Fingerprint confirmation (specs/022-fingerprint.md) through `fprintd-verify`, the
//! owning tool (P-07).

use std::fmt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::rules::find_tool;

/// What touching the sensor came to (FP-03, FP-04).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verified {
    Matched,
    NotInstalled,
    /// It ran and said no: no reader, no enrolled finger, no match, or its own timeout.
    NoMatch,
    CantStart(String),
}

impl fmt::Display for Verified {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Matched => f.write_str("the fingerprint matched"),
            Self::NotInstalled => f.write_str("fprintd-verify isn't installed"),
            Self::NoMatch => f.write_str("the fingerprint wasn't confirmed"),
            Self::CantStart(why) => write!(f, "fprintd-verify couldn't start: {why}"),
        }
    }
}

/// FP-03: `fprintd-verify` for the current user, no shell, stdin closed, output hidden.
/// `PATH` for it is only the absolute entries it was found on (SC-10).
pub fn verify(search_path: &[PathBuf]) -> Verified {
    let Some(program) = find_tool("fprintd-verify", search_path) else {
        return Verified::NotInstalled;
    };
    let status = Command::new(program)
        .env(
            "PATH",
            std::env::join_paths(search_path).unwrap_or_default(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => Verified::Matched,
        Ok(_) => Verified::NoMatch,
        Err(error) => Verified::CantStart(error.to_string()),
    }
}

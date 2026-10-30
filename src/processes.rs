//! Running programs, for P-08: a rule's data is left alone while its program runs.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[cfg(test)]
mod tests;

/// The names of every running program: each process's `comm` (cut to 15 bytes by the
/// kernel) and the base name of its executable from `cmdline`.
pub fn running() -> HashSet<String> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return HashSet::new();
    };
    let mut names = HashSet::new();
    for entry in entries.filter_map(Result::ok) {
        let dir = entry.path();
        // Processes come and go; one that vanished mid-read is simply not running.
        if let Ok(comm) = fs::read_to_string(dir.join("comm")) {
            names.insert(comm.trim_end().to_owned());
        }
        if let Some(name) = fs::read(dir.join("cmdline")).ok().and_then(|c| program(&c)) {
            names.insert(name);
        }
    }
    names
}

/// The base name of the first argument in a NUL-separated `cmdline`.
fn program(cmdline: &[u8]) -> Option<String> {
    let first = cmdline
        .split(|&b| b == 0)
        .next()
        .filter(|arg| !arg.is_empty())?;
    let name = Path::new(OsStr::from_bytes(first)).file_name()?;
    name.to_str().map(str::to_owned)
}

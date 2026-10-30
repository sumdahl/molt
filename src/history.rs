//! `molt history` (specs/017-history.md): the operation log (P-09, SD-15), read only.

use std::fmt;
use std::io;
use std::path::Path;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize, Serializer};

use crate::scan::human_size;

#[cfg(test)]
mod tests;

/// SD-15's outcomes, as the log writes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Outcome {
    Removed,
    Trashed,
    Failed,
}

/// What an entry did: an item went, or a command ran (CN-12, SY-11).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Action {
    Item {
        path: String,
        outcome: Outcome,
    },
    Command {
        command: Vec<String>,
        exit: Option<i32>,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct Line {
    time: String,
    bytes: u64,
    #[serde(flatten)]
    action: Action,
}

/// One entry, with the log's own line kept for `--json` (HI-07).
#[derive(Debug, Clone)]
pub struct Entry {
    time: Timestamp,
    bytes: u64,
    action: Action,
    raw: serde_json::Value,
}

impl Serialize for Entry {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(serializer)
    }
}

impl Entry {
    fn parse(line: &str) -> Option<Self> {
        let raw: serde_json::Value = serde_json::from_str(line).ok()?;
        let line = Line::deserialize(&raw).ok()?;
        Some(Self {
            time: line.time.parse().ok()?,
            bytes: line.bytes,
            action: line.action,
            raw,
        })
    }

    /// HI-02: local time to the minute, what happened, the size, and the path or command.
    fn line(&self, tz: &TimeZone) -> String {
        let when = self.time.to_zoned(tz.clone()).strftime("%Y-%m-%d %H:%M");
        let size = |bytes: u64| {
            if bytes > 0 {
                human_size(bytes)
            } else {
                String::new()
            }
        };
        match &self.action {
            Action::Item { path, outcome } => {
                let what = match outcome {
                    Outcome::Trashed => "trashed",
                    Outcome::Removed => "deleted",
                    Outcome::Failed => "partly deleted",
                };
                format!("{when}  {what:<14}  {:>9}  {path}", size(self.bytes))
            }
            Action::Command { command, exit } => {
                let status = match exit {
                    Some(0) => String::new(),
                    Some(code) => format!("  (exit {code})"),
                    None => "  (didn't run)".to_owned(),
                };
                let size = size(self.bytes);
                format!(
                    "{when}  {:<14}  {size:>9}  {}{status}",
                    "ran",
                    command.join(" ")
                )
            }
        }
    }

    fn trashed(&self) -> bool {
        matches!(
            self.action,
            Action::Item {
                outcome: Outcome::Trashed,
                ..
            }
        )
    }
}

/// Every readable entry in the log, in its order, and how many lines weren't (HI-06).
#[derive(Debug, Clone)]
pub struct History {
    entries: Vec<Entry>,
    skipped_lines: usize,
}

fn parse(text: &str) -> History {
    let mut history = History {
        entries: Vec::new(),
        skipped_lines: 0,
    };
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        match Entry::parse(line) {
            Some(entry) => history.entries.push(entry),
            None => history.skipped_lines += 1,
        }
    }
    history
}

/// HI-01, HI-06: reads `log` without writing anything. `None` when there is no log yet.
pub fn read(log: &Path) -> io::Result<Option<History>> {
    match std::fs::read_to_string(log) {
        Ok(text) => Ok(Some(parse(&text))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// The entries to show (HI-03, HI-04), newest first.
#[derive(Debug, Serialize)]
pub struct Shown {
    entries: Vec<Entry>,
    skipped_lines: usize,
    left_out: usize,
}

impl History {
    /// HI-03, HI-04: entries from `since` on, newest first, at most `limit`.
    pub fn select(self, since: Option<Timestamp>, limit: Option<usize>) -> Shown {
        let mut entries: Vec<Entry> = self
            .entries
            .into_iter()
            .filter(|entry| since.is_none_or(|since| entry.time >= since))
            .collect();
        // Stable, so entries with the same time keep the log's order, reversed.
        entries.reverse();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.time));
        let left_out = limit.map_or(0, |limit| entries.len().saturating_sub(limit));
        entries.truncate(entries.len() - left_out);
        Shown {
            entries,
            skipped_lines: self.skipped_lines,
            left_out,
        }
    }
}

impl Shown {
    /// HI-05
    fn freed(&self) -> u64 {
        self.entries.iter().map(|entry| entry.bytes).sum()
    }

    fn trashed(&self) -> usize {
        self.entries.iter().filter(|entry| entry.trashed()).count()
    }

    /// HI-06, P-13: partial when any line couldn't be read.
    pub fn exit_code(&self) -> u8 {
        if self.skipped_lines > 0 { 3 } else { 0 }
    }
}

impl fmt::Display for Shown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tz = TimeZone::system();
        for entry in &self.entries {
            writeln!(f, "{}", entry.line(&tz))?;
        }
        writeln!(f)?;
        write!(
            f,
            "Freed {} in {} entries",
            human_size(self.freed()),
            self.entries.len()
        )?;
        match self.trashed() {
            0 => writeln!(f, ".")?,
            n => writeln!(f, "; {n} went to the trash, where they can be restored.")?,
        }
        if self.left_out > 0 {
            writeln!(
                f,
                "{} older entries not shown; --all shows them.",
                self.left_out
            )?;
        }
        match self.skipped_lines {
            0 => {}
            1 => writeln!(f, "1 line in the log couldn't be read and was skipped.")?,
            n => writeln!(f, "{n} lines in the log couldn't be read and were skipped.")?,
        }
        Ok(())
    }
}

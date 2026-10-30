//! `molt scan` (specs/005-scan.md): what could be freed, measured by a safe-delete dry run.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::rules::{self, Category, Match, Rule, RuleId, SkipReason};
use crate::safe_delete::{
    self, Candidate, Context, Execution, Mode, Outcome, Permissions, Plan, PlanRefused, Privilege,
};
use crate::startup::Startup;

#[cfg(test)]
mod tests;

#[derive(Debug, Serialize)]
pub struct ScanReport {
    pub total: u64,
    /// P-12: some directory couldn't be read, so sizes may be higher.
    pub partial: bool,
    pub rules: Vec<RuleReport>,
    pub skipped: Vec<SkipEntry>,
    pub inactive: Vec<RuleId>,
}

#[derive(Debug, Serialize)]
pub struct RuleReport {
    pub id: RuleId,
    pub name: String,
    pub category: Category,
    pub mode: Mode,
    pub bytes: u64,
    pub clean_with: Vec<String>,
    pub items: Vec<Item>,
    /// LF-06: folders the user owns but can't read; their contents weren't measured.
    pub locked: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Item {
    pub path: String,
    pub bytes: u64,
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SkipEntry {
    pub rule: RuleId,
    pub path: String,
    pub reason: String,
}

/// Matches the enabled rules and measures them without changing anything (SC-01).
pub fn scan(startup: &Startup) -> Result<ScanReport, PlanRefused> {
    let env = &startup.env;
    let findings = rules::find(&startup.rules, &env.dirs, &env.search_path);
    let (outcomes, locked) = measure(&findings.matches, &env.context(&startup.config))?;

    let mut rules: Vec<RuleReport> = startup.rules.iter().map(RuleReport::new).collect();
    for (found, outcome) in findings.matches.iter().zip(outcomes) {
        if let Some(rule) = rules.iter_mut().find(|rule| rule.id == found.rule) {
            let path = found.candidate.path();
            rule.add(path, outcome);
            rule.locked.extend(
                locked
                    .iter()
                    .filter(|folder| folder.starts_with(path))
                    .map(|folder| folder.to_string_lossy().into_owned()),
            );
        }
    }
    // SC-03: rules with no matches are left out.
    rules.retain(|rule| !rule.items.is_empty());
    sort_largest_first(&mut rules);

    let unreadable = findings
        .skipped
        .iter()
        .any(|skip| matches!(skip.reason, SkipReason::Unreadable(_)));
    let failed = rules
        .iter()
        .flat_map(|rule| &rule.items)
        .any(|item| item.outcome == "failed");
    Ok(ScanReport {
        total: rules.iter().map(|rule| rule.bytes).sum(),
        partial: unreadable || failed,
        rules,
        skipped: findings
            .skipped
            .into_iter()
            .map(|skip| SkipEntry {
                rule: skip.rule,
                path: skip.path.to_string_lossy().into_owned(),
                reason: skip.reason.to_string(),
            })
            .collect(),
        inactive: findings.inactive,
    })
}

/// SC-02: a safe-delete dry run of each mode's candidates. Returns one outcome per match,
/// and the locked folders found (LF-06).
fn measure(matches: &[Match], ctx: &Context) -> Result<(Vec<Outcome>, Vec<PathBuf>), PlanRefused> {
    let mut locked = Vec::new();
    let mut outcomes = vec![None; matches.len()];
    for mode in [Mode::Permanent, Mode::Trash] {
        let (indices, candidates): (Vec<usize>, Vec<Candidate>) = matches
            .iter()
            .enumerate()
            .filter(|(_, found)| found.mode == mode)
            .map(|(i, found)| (i, found.candidate.clone()))
            .unzip();
        let plan = Plan {
            candidates,
            mode,
            execution: Execution::DryRun,
            privilege: Privilege::User,
            permissions: Permissions::Keep,
        };
        let report = safe_delete::execute(&plan, ctx)?;
        locked.extend(report.locked);
        for (i, (_, outcome)) in indices.into_iter().zip(report.results) {
            outcomes[i] = Some(outcome);
        }
    }
    Ok((outcomes.into_iter().flatten().collect(), locked))
}

/// SC-03: categories by total size, then rules within each, largest first.
fn sort_largest_first(rules: &mut [RuleReport]) {
    let mut totals: HashMap<Category, u64> = HashMap::new();
    for rule in rules.iter() {
        *totals.entry(rule.category).or_default() += rule.bytes;
    }
    rules.sort_by_key(|rule| {
        (
            Reverse(totals[&rule.category]),
            rule.category,
            Reverse(rule.bytes),
        )
    });
}

impl RuleReport {
    fn new(rule: &Rule) -> Self {
        Self {
            id: rule.id.clone(),
            name: rule.name.clone(),
            category: rule.category,
            mode: match rule.kind {
                rules::Kind::Regenerable => Mode::Permanent,
                rules::Kind::UserContent => Mode::Trash,
            },
            bytes: 0,
            clean_with: rule.clean_with.clone(),
            items: Vec::new(),
            locked: Vec::new(),
        }
    }

    fn add(&mut self, path: &Path, outcome: Outcome) {
        let item = Item::new(path, outcome);
        self.bytes += item.bytes;
        self.items.push(item);
    }

    fn count(&self, outcome: &str) -> usize {
        self.items
            .iter()
            .filter(|item| item.outcome == outcome)
            .count()
    }
}

impl Item {
    fn new(path: &Path, outcome: Outcome) -> Self {
        let (bytes, outcome, reason) = match outcome {
            Outcome::WouldRemove { bytes, .. } => (bytes, "would-remove", None),
            Outcome::WouldTrash { bytes } => (bytes, "would-trash", None),
            // A dry run never removes; listed for completeness.
            Outcome::Removed { bytes, .. } => (bytes, "removed", None),
            Outcome::Trashed { bytes } => (bytes, "trashed", None),
            // SC-02: what safe delete would skip isn't counted.
            Outcome::Skipped(reason) => (0, "skipped", Some(reason.to_string())),
            Outcome::Failed { errno, .. } => (0, "failed", Some(errno.to_string())),
        };
        Self {
            path: path.to_string_lossy().into_owned(),
            bytes,
            outcome,
            reason,
        }
    }
}

const UNITS: [&str; 6] = ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];

/// SC-04: binary units, one decimal from KiB up.
pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    // Display precision only; u64 to f64 loses nothing visible at one decimal.
    #[allow(clippy::cast_precision_loss)]
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    // Compare the rounded value, so 1023.96 KiB shows as 1.0 MiB, not 1024.0 KiB.
    while (value * 10.0).round() / 10.0 >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn category_label(category: Category) -> &'static str {
    match category {
        Category::DevTools => "Developer tools",
        Category::Browsers => "Browsers",
        Category::Apps => "Apps",
        Category::SystemUser => "System (your files)",
        Category::Trash => "Trash",
    }
}

impl fmt::Display for ScanReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.rules.is_empty() {
            writeln!(f, "Nothing to clean.")?;
        }
        for group in self.rules.chunk_by(|a, b| a.category == b.category) {
            let total: u64 = group.iter().map(|rule| rule.bytes).sum();
            writeln!(
                f,
                "{:<44}{:>10}",
                category_label(group[0].category),
                human_size(total)
            )?;
            for rule in group {
                write!(f, "  {:<42}{:>10}", rule.name, human_size(rule.bytes))?;
                if !rule.clean_with.is_empty() {
                    write!(f, "  via `{}`", rule.clean_with.join(" "))?;
                }
                // SC-02, LF-06
                for outcome in ["skipped", "failed"] {
                    match rule.count(outcome) {
                        0 => {}
                        n => write!(f, "  ({n} {outcome})")?,
                    }
                }
                if !rule.locked.is_empty() {
                    write!(f, "  ({} locked)", rule.locked.len())?;
                }
                writeln!(f)?;
            }
        }
        writeln!(f)?;
        if !self.inactive.is_empty() {
            writeln!(
                f,
                "{} rules inactive: the tools they clean up after aren't installed.",
                self.inactive.len()
            )?;
        }
        let locked: usize = self.rules.iter().map(|rule| rule.locked.len()).sum();
        if locked > 0 {
            writeln!(
                f,
                "{locked} locked folders: you own them but can't read them, so their contents \
                 weren't measured. `molt clean` will ask before opening them (u+rwx)."
            )?;
        }
        if self.partial {
            writeln!(
                f,
                "Partial scan: some folders couldn't be read or measured, so sizes may be higher. See --json."
            )?;
        }
        writeln!(
            f,
            "Total: {} can be freed with `molt clean`.",
            human_size(self.total)
        )
    }
}

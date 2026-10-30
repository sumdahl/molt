//! `molt clean` (specs/006-clean.md): carry out what the preview showed, and only that.

use std::collections::HashSet;
use std::fmt;
use std::process::{Command, Stdio};

use serde::Serialize;

use crate::rules::{self, Rule, RuleId};
use crate::safe_delete::{
    self, CommandLog, Execution, Mode, Outcome, Permissions, Plan, PlanRefused, Privilege,
};
use crate::scan::{Item, Survey, human_size, survey};
use crate::startup::Startup;

/// A rule left out because a program that owns its data is running (CN-03, P-08).
#[derive(Debug, Clone, Serialize)]
pub struct Busy {
    pub rule: RuleId,
    pub name: String,
    pub program: String,
}

/// CN-02: the rules `--only` and `--skip` select. `Err` names an id that isn't loaded.
pub fn select(rules: &[Rule], only: &[String], skip: &[String]) -> Result<Vec<Rule>, String> {
    if let Some(unknown) = only
        .iter()
        .chain(skip)
        .find(|id| !rules.iter().any(|rule| rule.id.to_string() == **id))
    {
        return Err(unknown.clone());
    }
    Ok(rules
        .iter()
        .filter(|rule| {
            let id = rule.id.to_string();
            (only.is_empty() || only.contains(&id)) && !skip.contains(&id)
        })
        .cloned()
        .collect())
}

/// CN-03: splits off the rules whose programs are running.
pub fn leave_out_busy(rules: Vec<Rule>, running: &HashSet<String>) -> (Vec<Rule>, Vec<Busy>) {
    let mut busy = Vec::new();
    let free = rules
        .into_iter()
        .filter(
            |rule| match rule.processes.iter().find(|p| running.contains(*p)) {
                Some(program) => {
                    busy.push(Busy {
                        rule: rule.id.clone(),
                        name: rule.name.clone(),
                        program: program.clone(),
                    });
                    false
                }
                None => true,
            },
        )
        .collect();
    (free, busy)
}

#[derive(Debug, Serialize)]
pub struct CommandRun {
    pub command: Vec<String>,
    /// `None` when the command couldn't start or was killed by a signal.
    pub exit: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct RuleResult {
    pub id: RuleId,
    pub name: String,
    pub freed: u64,
    /// Everything planned for this rule was done (CN-11).
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<CommandRun>,
    pub items: Vec<Item>,
}

#[derive(Debug, Serialize)]
pub struct CleanResult {
    pub freed: u64,
    pub rules: Vec<RuleResult>,
}

impl CleanResult {
    /// P-13: 3 when anything planned failed or was kept.
    pub fn exit_code(&self) -> u8 {
        if self.rules.iter().all(|rule| rule.ok) {
            0
        } else {
            3
        }
    }
}

/// CN-08, CN-09: runs owning-tool commands first, then one live plan per mode over the
/// candidates the preview measured. `rules` are the rules the preview covered.
pub fn act(
    preview: &Survey,
    rules: &[Rule],
    startup: &Startup,
    permissions: Permissions,
) -> Result<CleanResult, PlanRefused> {
    let ctx = startup.env.context(&startup.config);
    let mut log = CommandLog::open(&ctx)?;
    let mut results = Vec::new();

    for report in &preview.report.rules {
        let Some(rule) = rules.iter().find(|rule| rule.id == report.id) else {
            continue;
        };
        if !rule.clean_with.is_empty() {
            results.push(run_tool(rule, report.bytes, startup, &mut log)?);
        }
    }

    for mode in [Mode::Permanent, Mode::Trash] {
        let planned: Vec<_> = preview
            .measured
            .iter()
            .filter(|(found, outcome)| {
                found.mode == mode && is_planned(outcome) && deletes_files(rules, &found.rule)
            })
            .map(|(found, _)| found)
            .collect();
        if planned.is_empty() {
            continue;
        }
        let plan = Plan {
            candidates: planned
                .iter()
                .map(|found| found.candidate.clone())
                .collect(),
            mode,
            execution: Execution::Live,
            privilege: Privilege::User,
            permissions,
        };
        let report = safe_delete::execute(&plan, &ctx)?;
        for (found, (path, outcome)) in planned.iter().zip(report.results) {
            let result = rule_result(&mut results, rules, &found.rule);
            result.ok &= is_done(&outcome);
            let item = Item::new(&path, outcome);
            result.freed += item.bytes;
            result.items.push(item);
        }
    }

    Ok(CleanResult {
        freed: results.iter().map(|rule| rule.freed).sum(),
        rules: results,
    })
}

/// What the preview said a live run would act on.
fn is_planned(outcome: &Outcome) -> bool {
    matches!(
        outcome,
        Outcome::WouldRemove { .. } | Outcome::WouldTrash { .. }
    )
}

fn is_done(outcome: &Outcome) -> bool {
    matches!(
        outcome,
        Outcome::Removed { partial: false, .. } | Outcome::Trashed { .. }
    )
}

/// A rule with an owning-tool command never has its files deleted by molt (CN-08).
fn deletes_files(rules: &[Rule], id: &RuleId) -> bool {
    rules
        .iter()
        .any(|rule| rule.id == *id && rule.clean_with.is_empty())
}

fn rule_result<'a>(
    results: &'a mut Vec<RuleResult>,
    rules: &[Rule],
    id: &RuleId,
) -> &'a mut RuleResult {
    let index = match results.iter().position(|result| result.id == *id) {
        Some(index) => index,
        None => {
            let name = rules
                .iter()
                .find(|rule| rule.id == *id)
                .map(|rule| rule.name.clone())
                .unwrap_or_default();
            results.push(RuleResult {
                id: id.clone(),
                name,
                freed: 0,
                ok: true,
                command: None,
                items: Vec::new(),
            });
            results.len() - 1
        }
    };
    &mut results[index]
}

/// CN-08: runs the rule's own cleanup command without a shell, with stdin closed and its
/// output on stderr (stdout stays molt's, for `--json`). Freed is the size before minus
/// after.
fn run_tool(
    rule: &Rule,
    before: u64,
    startup: &Startup,
    log: &mut CommandLog,
) -> Result<RuleResult, PlanRefused> {
    let search_path = &startup.env.search_path;
    let exit = match rules::find_tool(&rule.clean_with[0], search_path) {
        None => None,
        Some(program) => Command::new(program)
            .args(&rule.clean_with[1..])
            .env(
                "PATH",
                std::env::join_paths(search_path).unwrap_or_default(),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::from(std::io::stderr()))
            .status()
            .ok()
            .and_then(|status| status.code()),
    };
    let ok = exit == Some(0);
    let after = survey(std::slice::from_ref(rule), startup, None)?
        .report
        .total;
    let freed = before.saturating_sub(after);
    // SD-15: a log write that fails after the command ran stops the run.
    log.record(&rule.clean_with, exit, freed).map_err(|e| {
        PlanRefused::OpLog(safe_delete::Errno::from_io_error(&e).unwrap_or(safe_delete::Errno::IO))
    })?;
    Ok(RuleResult {
        id: rule.id.clone(),
        name: rule.name.clone(),
        freed,
        ok,
        command: Some(CommandRun {
            command: rule.clean_with.clone(),
            exit,
        }),
        items: Vec::new(),
    })
}

impl fmt::Display for CleanResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for rule in &self.rules {
            write!(f, "  {:<42}{:>10}", rule.name, human_size(rule.freed))?;
            match &rule.command {
                Some(run) if run.exit == Some(0) => write!(f, "  ran `{}`", run.command.join(" "))?,
                Some(run) => write!(
                    f,
                    "  `{}` failed ({}); its files were left alone",
                    run.command.join(" "),
                    run.exit
                        .map_or("didn't run".to_owned(), |code| format!("exit {code}"))
                )?,
                None if !rule.ok => {
                    let done = rule
                        .items
                        .iter()
                        .filter(|item| {
                            !item.partial && matches!(item.outcome, "removed" | "trashed")
                        })
                        .count();
                    let kept = rule.items.len() - done;
                    write!(f, "  ({kept} kept or failed; see --json)")?;
                }
                None => {}
            }
            writeln!(f)?;
        }
        writeln!(f, "Freed {}.", human_size(self.freed))
    }
}

//! `molt optimize` (specs/015-optimize.md): upkeep through each tool's own command, run
//! with `sudo` after the full preview (P-04, P-07, P-14).

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Serialize;

use crate::rules::find_tool;
use crate::safe_delete::CommandLog;
use crate::system::{CommandRun, Invocation, run_live};
use crate::uninstall::package::plain;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskId {
    PacmanOrphans,
    AptAutoremove,
    DnfAutoremove,
    ZypperUnneeded,
}

impl TaskId {
    /// OP-08: the order they run in.
    pub const ALL: [Self; 4] = [
        Self::PacmanOrphans,
        Self::AptAutoremove,
        Self::DnfAutoremove,
        Self::ZypperUnneeded,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::PacmanOrphans => "Orphan packages (pacman)",
            Self::AptAutoremove => "Unneeded packages (apt)",
            Self::DnfAutoremove => "Unneeded packages (dnf)",
            Self::ZypperUnneeded => "Unneeded packages (zypper)",
        }
    }

    fn tool(self) -> &'static str {
        match self {
            Self::PacmanOrphans => "pacman",
            Self::AptAutoremove => "apt-get",
            Self::DnfAutoremove => "dnf",
            Self::ZypperUnneeded => "zypper",
        }
    }

    /// OP-03: the read-only query that lists the packages.
    fn query(self) -> &'static [&'static str] {
        match self {
            Self::PacmanOrphans => &["-Qdtq"],
            Self::AptAutoremove => &["-s", "autoremove"],
            // dnf 4 ends each with a newline and dnf 5 doesn't, so a space too.
            Self::DnfAutoremove => &["repoquery", "--unneeded", "--qf", "%{name} "],
            Self::ZypperUnneeded => &["--quiet", "packages", "--unneeded"],
        }
    }

    /// The package names in the query's output, each once, in order.
    fn packages(self, output: &str) -> Vec<String> {
        let names: Vec<&str> = match self {
            Self::PacmanOrphans | Self::DnfAutoremove => output.split_whitespace().collect(),
            Self::AptAutoremove => output
                .lines()
                .filter_map(|line| line.strip_prefix("Remv "))
                .filter_map(|rest| rest.split_whitespace().next())
                .collect(),
            // A table: `S | Repository | Name | Version | Arch`, one row per version.
            Self::ZypperUnneeded => output
                .lines()
                .filter_map(|line| line.split('|').nth(2))
                .map(str::trim)
                .filter(|name| *name != "Name")
                .collect(),
        };
        let mut unique: Vec<String> = Vec::new();
        for name in names {
            if !unique.iter().any(|seen| seen == name) {
                unique.push(name.to_owned());
            }
        }
        unique
    }

    /// OP-06: names exactly `packages` where the manager takes names, and never `-y` or
    /// `--noconfirm`, so the manager asks for itself.
    fn live(self, tool: &Path, packages: &[String]) -> Invocation {
        let args: &[&str] = match self {
            Self::PacmanOrphans => &["-Rns"],
            // apt takes no names: its list is the one the preview showed.
            Self::AptAutoremove => return invocation(tool, &["autoremove"], &[]),
            Self::DnfAutoremove => &["remove"],
            Self::ZypperUnneeded => &["remove", "--clean-deps"],
        };
        invocation(tool, args, packages)
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PacmanOrphans => "pacman-orphans",
            Self::AptAutoremove => "apt-autoremove",
            Self::DnfAutoremove => "dnf-autoremove",
            Self::ZypperUnneeded => "zypper-unneeded",
        })
    }
}

fn invocation(tool: &Path, args: &[&str], packages: &[String]) -> Invocation {
    Invocation {
        program: tool.to_owned(),
        args: args
            .iter()
            .map(|arg| (*arg).to_owned())
            .chain(packages.iter().cloned())
            .collect(),
    }
}

/// An active task: its tool found on the absolute `$PATH` entries (OP-02).
#[derive(Debug, Clone)]
pub struct Task {
    pub id: TaskId,
    tool: PathBuf,
}

#[derive(Debug)]
pub struct Tasks {
    pub sudo: Option<PathBuf>,
    pub active: Vec<Task>,
    pub inactive: Vec<TaskId>,
}

/// OP-02: without `sudo`, every task is inactive.
pub fn detect(search_path: &[PathBuf]) -> Tasks {
    let sudo = find_tool("sudo", search_path);
    let (mut active, mut inactive) = (Vec::new(), Vec::new());
    for id in TaskId::ALL {
        match (&sudo, find_tool(id.tool(), search_path)) {
            (Some(_), Some(tool)) => active.push(Task { id, tool }),
            _ => inactive.push(id),
        }
    }
    Tasks {
        sudo,
        active,
        inactive,
    }
}

/// OP-01: narrows the active tasks. `Err` names an id that isn't a task.
pub fn select(tasks: Vec<Task>, only: &[String], skip: &[String]) -> Result<Vec<Task>, String> {
    let known = |id: &String| TaskId::ALL.iter().any(|task| task.to_string() == *id);
    if let Some(unknown) = only.iter().chain(skip).find(|id| !known(id)) {
        return Err(unknown.clone());
    }
    Ok(tasks
        .into_iter()
        .filter(|task| {
            let id = task.id.to_string();
            (only.is_empty() || only.contains(&id)) && !skip.contains(&id)
        })
        .collect())
}

/// What a task's query found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Found {
    Packages(Vec<String>),
    Nothing,
    /// OP-05: its exit status, `None` when it couldn't start or was killed.
    Failed(Option<i32>),
    /// OP-04: the first name that isn't one plain package name.
    Unsafe(String),
}

/// OP-04, OP-05: reads a finished query.
fn found(id: TaskId, exit: Option<i32>, stdout: &str) -> Found {
    let packages = id.packages(stdout);
    // pacman -Qdtq exits 1, printing nothing, when there are no orphans.
    let none_from_pacman = id == TaskId::PacmanOrphans && exit == Some(1) && packages.is_empty();
    if none_from_pacman {
        return Found::Nothing;
    }
    if exit != Some(0) {
        return Found::Failed(exit);
    }
    if let Some(bad) = packages.iter().find(|name| !plain(name)) {
        return Found::Unsafe(bad.clone());
    }
    if packages.is_empty() {
        Found::Nothing
    } else {
        Found::Packages(packages)
    }
}

#[derive(Debug, Serialize)]
pub struct TaskPreview {
    pub id: TaskId,
    pub name: &'static str,
    pub query: String,
    pub found: Found,
    /// The exact command a live run runs, when there's anything to remove.
    pub would_run: Option<String>,
    #[serde(skip)]
    live: Option<Invocation>,
}

#[derive(Debug, Serialize)]
pub struct Preview {
    pub tasks: Vec<TaskPreview>,
    pub inactive: Vec<TaskId>,
}

impl Preview {
    /// Whether a live run would run anything.
    pub fn has_work(&self) -> bool {
        self.tasks.iter().any(|task| task.live.is_some())
    }
}

/// OP-03: runs each query without `sudo`, no shell, stdin closed; stderr passes through.
pub fn preview(tasks: &Tasks, selected: &[Task], search_path: &[PathBuf]) -> Preview {
    let previews = selected
        .iter()
        .map(|task| {
            let query = invocation(&task.tool, task.id.query(), &[]);
            let output = query
                .command(search_path)
                .stdin(Stdio::null())
                .stderr(Stdio::inherit())
                .output()
                .ok();
            let found = match &output {
                Some(output) => found(
                    task.id,
                    output.status.code(),
                    &String::from_utf8_lossy(&output.stdout),
                ),
                None => Found::Failed(None),
            };
            let live = match (&found, &tasks.sudo) {
                (Found::Packages(packages), Some(sudo)) => {
                    Some(task.id.live(&task.tool, packages).under(sudo))
                }
                _ => None,
            };
            TaskPreview {
                id: task.id,
                name: task.id.name(),
                query: query.to_string(),
                found,
                would_run: live.as_ref().map(ToString::to_string),
                live,
            }
        })
        .collect();
    Preview {
        tasks: previews,
        inactive: tasks.inactive.clone(),
    }
}

#[derive(Debug, Serialize)]
pub struct OptimizeResult {
    pub commands: Vec<CommandRun>,
}

impl OptimizeResult {
    /// OP-08, P-13
    pub fn exit_code(&self) -> u8 {
        if self.commands.iter().all(|run| run.exit == Some(0)) {
            0
        } else {
            3
        }
    }
}

/// OP-06, OP-08: runs exactly what the preview showed, in order, logging each command
/// (SY-11). A failed command never stops the next.
pub fn act(
    preview: &Preview,
    search_path: &[PathBuf],
    log: &mut CommandLog,
) -> std::io::Result<OptimizeResult> {
    let mut commands = Vec::new();
    for run in preview.tasks.iter().filter_map(|task| task.live.as_ref()) {
        let exit = run_live(run, search_path);
        log.record(&run.words(), exit, 0)?;
        commands.push(CommandRun {
            command: run.to_string(),
            exit,
            summary: None,
        });
    }
    Ok(OptimizeResult { commands })
}

impl fmt::Display for Preview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.tasks.is_empty() {
            writeln!(f, "No optimize tasks to run.")?;
        }
        for task in &self.tasks {
            writeln!(f, "  {}", task.name)?;
            match &task.found {
                Found::Packages(packages) => {
                    for package in packages {
                        writeln!(f, "    {package}")?;
                    }
                }
                Found::Nothing => writeln!(f, "    nothing to remove")?,
                Found::Failed(exit) => {
                    let exit = exit.map_or("didn't run".to_owned(), |code| format!("exit {code}"));
                    writeln!(f, "    `{}` failed ({exit})", task.query)?;
                }
                Found::Unsafe(name) => {
                    writeln!(f, "    left out: {name:?} isn't one plain package name")?;
                }
            }
            if let Some(command) = &task.would_run {
                writeln!(f, "    would run: {command}")?;
            }
        }
        if !self.inactive.is_empty() {
            let ids: Vec<String> = self.inactive.iter().map(ToString::to_string).collect();
            writeln!(
                f,
                "\nInactive, as their tools (or sudo) aren't installed: {}",
                ids.join(", ")
            )?;
        }
        Ok(())
    }
}

impl fmt::Display for OptimizeResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for run in &self.commands {
            let exit = run
                .exit
                .map_or("didn't run".to_owned(), |code| format!("exit {code}"));
            writeln!(f, "  {}  ({exit})", run.command)?;
        }
        Ok(())
    }
}

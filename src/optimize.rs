//! `onyx optimize` (specs/015-optimize.md): upkeep through each tool's own command, run
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
    /// OP-09 to OP-14: maintenance, after the packages.
    Fstrim,
    FontCache,
    DnsCache,
    /// OP-15 to OP-20: read-only checks, last.
    FailedUnits,
    Smart,
}

impl TaskId {
    /// OP-08, OP-14: the order they run in.
    pub const ALL: [Self; 9] = [
        Self::PacmanOrphans,
        Self::AptAutoremove,
        Self::DnfAutoremove,
        Self::ZypperUnneeded,
        Self::Fstrim,
        Self::FontCache,
        Self::DnsCache,
        Self::FailedUnits,
        Self::Smart,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::PacmanOrphans => "Orphan packages (pacman)",
            Self::AptAutoremove => "Unneeded packages (apt)",
            Self::DnfAutoremove => "Unneeded packages (dnf)",
            Self::ZypperUnneeded => "Unneeded packages (zypper)",
            Self::Fstrim => "TRIM: tell each SSD which blocks no file uses",
            Self::FontCache => "Font cache: rebuild your out-of-date caches",
            Self::DnsCache => "DNS cache: forget remembered answers",
            Self::FailedUnits => "Failed systemd units",
            Self::Smart => "Disk health (SMART)",
        }
    }

    /// OP-11, OP-15: your own font cache, and the checks, need no root.
    fn needs_sudo(self) -> bool {
        !matches!(self, Self::FontCache | Self::FailedUnits | Self::Smart)
    }

    fn tool(self) -> &'static str {
        match self {
            Self::PacmanOrphans => "pacman",
            Self::AptAutoremove => "apt-get",
            Self::DnfAutoremove => "dnf",
            Self::ZypperUnneeded => "zypper",
            Self::Fstrim => "fstrim",
            Self::FontCache => "fc-cache",
            Self::DnsCache => "resolvectl",
            Self::FailedUnits => "systemctl",
            Self::Smart => "smartctl",
        }
    }

    /// OP-03, OP-10: the read-only query, if the task has one.
    fn query(self) -> Option<&'static [&'static str]> {
        Some(match self {
            Self::PacmanOrphans => &["-Qdtq"],
            Self::AptAutoremove => &["-s", "autoremove"],
            // dnf 4 ends each with a newline and dnf 5 doesn't, so a space too.
            Self::DnfAutoremove => &["repoquery", "--unneeded", "--qf", "%{name} "],
            Self::ZypperUnneeded => &["--quiet", "packages", "--unneeded"],
            // 015 Decisions: `fstrim --dry-run` sees no filesystems without root.
            // The checks run several queries of their own (`check`).
            Self::Fstrim | Self::FontCache | Self::FailedUnits | Self::Smart => return None,
            // OP-12: it also tells whether systemd-resolved is running.
            Self::DnsCache => &["statistics"],
        })
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
            Self::Fstrim | Self::FontCache | Self::DnsCache | Self::FailedUnits | Self::Smart => {
                Vec::new()
            }
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
            // OP-13: no --force or -f, and no paths from onyx.
            Self::Fstrim => &["--all", "--verbose"],
            Self::FontCache => &[],
            Self::DnsCache => &["flush-caches"],
            // OP-15: checks have no live command; `preview` never asks for one.
            Self::FailedUnits | Self::Smart => &[],
        };
        invocation(tool, args, packages)
    }
}

impl TaskId {
    /// The id `--only` and `--skip` take, and completions offer (CO-03).
    pub fn id(self) -> &'static str {
        match self {
            Self::PacmanOrphans => "pacman-orphans",
            Self::AptAutoremove => "apt-autoremove",
            Self::DnfAutoremove => "dnf-autoremove",
            Self::ZypperUnneeded => "zypper-unneeded",
            Self::Fstrim => "fstrim",
            Self::FontCache => "font-cache",
            Self::DnsCache => "dns-cache",
            Self::FailedUnits => "failed-units",
            Self::Smart => "smart",
        }
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
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

/// OP-02, OP-11: without `sudo`, every task that needs it is inactive.
pub fn detect(search_path: &[PathBuf]) -> Tasks {
    let sudo = find_tool("sudo", search_path);
    let (mut active, mut inactive) = (Vec::new(), Vec::new());
    for id in TaskId::ALL {
        match find_tool(id.tool(), search_path) {
            Some(tool) if sudo.is_some() || !id.needs_sudo() => active.push(Task { id, tool }),
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
    /// OP-10: a maintenance task that will run, with its preview's line if it has one.
    Ready(Option<String>),
    /// OP-20: what a check found, line by line.
    Report(Vec<String>),
}

/// OP-04, OP-05, OP-12: reads a finished query.
fn found(id: TaskId, exit: Option<i32>, stdout: &str) -> Found {
    if id == TaskId::DnsCache {
        return match exit {
            Some(0) => Found::Ready(cache_size(stdout)),
            _ => Found::Failed(exit),
        };
    }
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

/// OP-12: `resolvectl statistics`' "Current Cache Size" counts answers, not bytes.
fn cache_size(stats: &str) -> Option<String> {
    let line = stats
        .lines()
        .find(|line| line.contains("Current Cache Size"))?;
    let count: u64 = line.rsplit(':').next()?.trim().parse().ok()?;
    Some(match count {
        1 => "1 answer cached".to_owned(),
        n => format!("{n} answers cached"),
    })
}

#[derive(Debug, Serialize)]
pub struct TaskPreview {
    pub id: TaskId,
    pub name: &'static str,
    pub query: Option<String>,
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

    /// OP-09: the one question, naming the tasks that use `sudo`, since some don't (#85).
    pub fn question(&self) -> String {
        let sudo: Vec<&str> = self
            .tasks
            .iter()
            .filter(|task| task.live.is_some() && task.id.needs_sudo())
            .map(|task| task.id.id())
            .collect();
        match sudo.as_slice() {
            [] => "Run all of the above?".to_owned(),
            [one] => format!("Run all of the above? {one} uses sudo."),
            many => format!("Run all of the above? {} use sudo.", many.join(", ")),
        }
    }
}

/// OP-03: runs each query without `sudo`, no shell, stdin closed; stderr passes through.
pub fn preview(tasks: &Tasks, selected: &[Task], search_path: &[PathBuf]) -> Preview {
    let previews = selected
        .iter()
        .map(|task| {
            let query = task
                .id
                .query()
                .map(|args| invocation(&task.tool, args, &[]));
            let found = match (&query, task.id) {
                (_, TaskId::FailedUnits | TaskId::Smart) => {
                    Found::Report(check(task, search_path, Path::new("/sys/block")))
                }
                (Some(query), _) => run_query(task.id, query, search_path),
                (None, _) => Found::Ready(None),
            };
            let live = match &found {
                Found::Packages(packages) => Some(task.id.live(&task.tool, packages)),
                Found::Ready(_) => Some(task.id.live(&task.tool, &[])),
                _ => None,
            }
            .and_then(|run| match (task.id.needs_sudo(), &tasks.sudo) {
                (false, _) => Some(run),
                (true, Some(sudo)) => Some(run.under(sudo)),
                (true, None) => None,
            });
            TaskPreview {
                id: task.id,
                name: task.id.name(),
                query: query.as_ref().map(ToString::to_string),
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

fn run_query(id: TaskId, query: &Invocation, search_path: &[PathBuf]) -> Found {
    match query
        .command(search_path)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
    {
        Ok(output) => found(
            id,
            output.status.code(),
            &String::from_utf8_lossy(&output.stdout),
        ),
        Err(_) => Found::Failed(None),
    }
}

/// OP-15: runs a check's read-only queries: no sudo, no shell, stdin closed.
fn check(task: &Task, search_path: &[PathBuf], sys_block: &Path) -> Vec<String> {
    let ask = |args: &[&str]| {
        invocation(&task.tool, args, &[])
            .command(search_path)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .map(|out| {
                (
                    out.status.code(),
                    String::from_utf8_lossy(&out.stdout).into_owned(),
                )
            })
    };
    match task.id {
        TaskId::FailedUnits => failed_unit_lines(
            ask(&["--failed", "--plain", "--no-legend"]),
            ask(&["--user", "--failed", "--plain", "--no-legend"]),
        ),
        _ => disks(sys_block)
            .iter()
            .map(|disk| {
                let device = format!("/dev/{disk}");
                // OP-17: `-H` and the device, nothing else.
                match ask(&["-H", &device]) {
                    Some((exit, stdout)) => smart_health(disk, exit, &stdout),
                    None => format!("{disk}: smartctl didn't run"),
                }
            })
            .collect(),
    }
}

const SYSTEMCTL_FAILED: &str = "systemctl --failed --plain --no-legend";

/// OP-16: each failed unit, the user's marked, then where to look closer.
fn failed_unit_lines(
    system: Option<(Option<i32>, String)>,
    user: Option<(Option<i32>, String)>,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut units = 0;
    for (answer, scope, mark) in [(system, "", ""), (user, " --user", " (user)")] {
        match answer {
            Some((Some(0), stdout)) => {
                for unit in stdout
                    .lines()
                    .filter_map(|line| line.split_whitespace().next())
                {
                    lines.push(format!("{unit}{mark}"));
                    units += 1;
                }
            }
            Some((exit, _)) => {
                let exit = exit.map_or("didn't run".to_owned(), |code| format!("exit {code}"));
                lines.push(format!("`{SYSTEMCTL_FAILED}{scope}` failed ({exit})"));
            }
            None => lines.push(format!("`{SYSTEMCTL_FAILED}{scope}` didn't run")),
        }
    }
    if units > 0 {
        lines.push(
            "look closer: systemctl status <unit>, journalctl -b -u <unit> (--user for yours)"
                .to_owned(),
        );
    } else if lines.is_empty() {
        lines.push("no failed units".to_owned());
    }
    lines
}

/// OP-17: whole disks in `sys_block`, without virtual ones, with plain names, sorted.
fn disks(sys_block: &Path) -> Vec<String> {
    const VIRTUAL: [&str; 8] = ["loop", "ram", "zram", "dm-", "md", "sr", "fd", "nbd"];
    let Ok(entries) = std::fs::read_dir(sys_block) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric()))
        .filter(|name| !VIRTUAL.iter().any(|prefix| name.starts_with(prefix)))
        .collect();
    names.sort();
    names
}

/// OP-18: one disk's line from smartctl's verdict and exit status bits.
fn smart_health(disk: &str, exit: Option<i32>, stdout: &str) -> String {
    let verdict = stdout
        .lines()
        .find(|line| line.contains("test result:") || line.contains("Health Status:"))
        .and_then(|line| line.split_once(':'))
        .map(|(_, value)| value.trim());
    let bits = exit.unwrap_or(0);
    let health = if verdict.is_some_and(|v| v.starts_with("FAILED")) || bits & 8 != 0 {
        "FAILING".to_owned()
    } else if verdict.is_some_and(|v| v.starts_with("PASSED") || v.starts_with("OK")) {
        "passed".to_owned()
    } else if bits & 2 != 0 {
        format!("needs root: run `sudo smartctl -H /dev/{disk}`")
    } else {
        "no SMART data".to_owned()
    };
    format!("{disk}: {health}")
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
                    let query = task.query.as_deref().unwrap_or_default();
                    writeln!(f, "    left out: `{query}` failed ({exit})")?;
                }
                Found::Unsafe(name) => {
                    writeln!(f, "    left out: {name:?} isn't one plain package name")?;
                }
                Found::Report(lines) => {
                    for line in lines {
                        writeln!(f, "    {line}")?;
                    }
                }
                Found::Ready(line) => {
                    if let Some(line) = line {
                        writeln!(f, "    preview: {line}")?;
                    }
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

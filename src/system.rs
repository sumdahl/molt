//! `molt system` (specs/007-system.md): the one privileged step (P-04). molt runs owning
//! tools' commands through `sudo`, and never deletes anything as root itself.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

use crate::rules::find_tool;
use crate::safe_delete::CommandLog;
use crate::scan::human_size;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskId {
    PacmanCache,
    Journal,
    Coredumps,
}

impl TaskId {
    pub const ALL: [Self; 3] = [Self::PacmanCache, Self::Journal, Self::Coredumps];

    fn name(self) -> &'static str {
        match self {
            Self::PacmanCache => "pacman package cache",
            Self::Journal => "systemd journal",
            Self::Coredumps => "Coredumps",
        }
    }

    /// The folders the task's size is measured in (SY-09).
    fn folders(self) -> &'static [&'static str] {
        match self {
            Self::PacmanCache => &["/var/cache/pacman/pkg"],
            Self::Journal => &["/var/log/journal", "/run/log/journal"],
            Self::Coredumps => &["/var/lib/systemd/coredump"],
        }
    }

    /// Read-only commands, run without `sudo` (SY-04).
    fn previews(self) -> &'static [&'static [&'static str]] {
        match self {
            Self::PacmanCache => &[&["paccache", "-dk2"], &["paccache", "-duk0"]],
            Self::Journal => &[&["journalctl", "--disk-usage"]],
            Self::Coredumps => &[&[
                "systemd-tmpfiles",
                "--clean",
                "--dry-run",
                "--prefix",
                "/var/lib/systemd/coredump",
            ]],
        }
    }

    /// What a live run runs through `sudo` (SY-07).
    fn commands(self) -> &'static [&'static [&'static str]] {
        match self {
            Self::PacmanCache => &[&["paccache", "-rk2"], &["paccache", "-ruk0"]],
            Self::Journal => &[&["journalctl", "--vacuum-size=500M"]],
            Self::Coredumps => &[&[
                "systemd-tmpfiles",
                "--clean",
                "--prefix",
                "/var/lib/systemd/coredump",
            ]],
        }
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PacmanCache => "pacman-cache",
            Self::Journal => "journal",
            Self::Coredumps => "coredumps",
        })
    }
}

/// A program found on the absolute `$PATH` entries, and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl Invocation {
    pub(crate) fn find(words: &[&str], search_path: &[PathBuf]) -> Option<Self> {
        let (tool, args) = words.split_first()?;
        Some(Self {
            program: find_tool(tool, search_path)?,
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        })
    }

    /// SY-07: `sudo -- <absolute tool path> <args>`, so sudo runs what the preview showed.
    pub(crate) fn under(&self, sudo: &Path) -> Self {
        let mut args = vec!["--".to_owned(), self.program.to_string_lossy().into_owned()];
        args.extend(self.args.iter().cloned());
        Self {
            program: sudo.to_owned(),
            args,
        }
    }

    pub(crate) fn words(&self) -> Vec<String> {
        std::iter::once(self.program.to_string_lossy().into_owned())
            .chain(self.args.iter().cloned())
            .collect()
    }

    pub(crate) fn command(&self, search_path: &[PathBuf]) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args).env(
            "PATH",
            std::env::join_paths(search_path).unwrap_or_default(),
        );
        command
    }
}

impl fmt::Display for Invocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.words().join(" "))
    }
}

/// An active task, with every tool it needs found (SY-02).
#[derive(Debug, Clone)]
pub struct Task {
    pub id: TaskId,
    previews: Vec<Invocation>,
    commands: Vec<Invocation>,
}

/// The tasks whose tools are found, and the ids of those that aren't (SY-02).
#[derive(Debug)]
pub struct Tasks {
    pub sudo: Option<PathBuf>,
    pub active: Vec<Task>,
    pub inactive: Vec<TaskId>,
}

pub fn detect(search_path: &[PathBuf]) -> Tasks {
    let sudo = find_tool("sudo", search_path);
    let mut tasks = Tasks {
        sudo,
        active: Vec::new(),
        inactive: Vec::new(),
    };
    for id in TaskId::ALL {
        let find = |list: &[&[&str]]| -> Option<Vec<Invocation>> {
            list.iter()
                .map(|words| Invocation::find(words, search_path))
                .collect()
        };
        match (&tasks.sudo, find(id.previews()), find(id.commands())) {
            (Some(_), Some(previews), Some(commands)) => tasks.active.push(Task {
                id,
                previews,
                commands,
            }),
            _ => tasks.inactive.push(id),
        }
    }
    tasks
}

/// SY-01: narrows the active tasks. `Err` names an id that isn't a task.
pub fn select(tasks: Vec<Task>, only: &[String], skip: &[String]) -> Result<Vec<Task>, String> {
    if let Some(unknown) = only
        .iter()
        .chain(skip)
        .find(|id| !TaskId::ALL.iter().any(|task| task.to_string() == **id))
    {
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

#[derive(Debug, Serialize)]
pub struct CommandRun {
    pub command: String,
    /// `None` when the command couldn't start or was killed by a signal.
    pub exit: Option<i32>,
    /// The last line it printed (SY-03).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TaskPreview {
    pub id: TaskId,
    pub name: &'static str,
    pub bytes: u64,
    pub previews: Vec<CommandRun>,
    /// The exact command lines a live run runs.
    pub would_run: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Preview {
    pub tasks: Vec<TaskPreview>,
    pub inactive: Vec<TaskId>,
}

/// SY-03, SY-04: measures each task under `root` and runs its read-only previews.
pub fn preview(tasks: &Tasks, selected: &[Task], root: &Path, search_path: &[PathBuf]) -> Preview {
    let tasks_preview = selected
        .iter()
        .map(|task| TaskPreview {
            id: task.id,
            name: task.id.name(),
            bytes: size(task.id, root),
            previews: task
                .previews
                .iter()
                .map(|run| run_preview(run, search_path))
                .collect(),
            would_run: match &tasks.sudo {
                Some(sudo) => task
                    .commands
                    .iter()
                    .map(|run| run.under(sudo).to_string())
                    .collect(),
                None => Vec::new(),
            },
        })
        .collect();
    Preview {
        tasks: tasks_preview,
        inactive: tasks.inactive.clone(),
    }
}

/// SY-04: no shell, no sudo, stdin closed; the output is echoed to stderr.
fn run_preview(run: &Invocation, search_path: &[PathBuf]) -> CommandRun {
    let output = run.command(search_path).stdin(Stdio::null()).output().ok();
    if let Some(output) = &output {
        eprint!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    CommandRun {
        command: run.to_string(),
        exit: output.as_ref().and_then(|output| output.status.code()),
        summary: output
            .and_then(|output| last_line(&output.stdout).or_else(|| last_line(&output.stderr))),
    }
}

fn last_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(str::to_owned)
}

#[derive(Debug, Serialize)]
pub struct TaskResult {
    pub id: TaskId,
    pub name: &'static str,
    pub freed: u64,
    pub commands: Vec<CommandRun>,
}

impl TaskResult {
    fn ok(&self) -> bool {
        self.commands.iter().all(|run| run.exit == Some(0))
    }
}

#[derive(Debug, Serialize)]
pub struct SystemResult {
    pub freed: u64,
    pub tasks: Vec<TaskResult>,
}

impl SystemResult {
    /// SY-10, P-13
    pub fn exit_code(&self) -> u8 {
        if self.tasks.iter().all(TaskResult::ok) {
            0
        } else {
            3
        }
    }
}

/// SY-07, SY-08, SY-09, SY-11: runs each task's commands through `sudo`, in order. A
/// failed task never stops the next; a failed log write stops the run.
pub fn act(
    sudo: &Path,
    selected: &[Task],
    root: &Path,
    search_path: &[PathBuf],
    log: &mut CommandLog,
) -> std::io::Result<SystemResult> {
    let mut results = Vec::new();
    for task in selected {
        let before = size(task.id, root);
        let mut commands = Vec::new();
        for run in &task.commands {
            let run = run.under(sudo);
            let exit = run_live(&run, search_path);
            commands.push((run, exit));
            if exit != Some(0) {
                break;
            }
        }
        let freed = before.saturating_sub(size(task.id, root));
        for (run, exit) in &commands {
            log.record(&run.words(), *exit, freed)?;
        }
        results.push(TaskResult {
            id: task.id,
            name: task.id.name(),
            freed,
            commands: commands
                .into_iter()
                .map(|(run, exit)| CommandRun {
                    command: run.to_string(),
                    exit,
                    summary: None,
                })
                .collect(),
        });
    }
    Ok(SystemResult {
        freed: results.iter().map(|task| task.freed).sum(),
        tasks: results,
    })
}

/// SY-07: runs `run` with stdin still the terminal, for sudo's password prompt, and its
/// output on stderr. `None` when it couldn't start or was killed by a signal.
pub(crate) fn run_live(run: &Invocation, search_path: &[PathBuf]) -> Option<i32> {
    run.command(search_path)
        .stdout(Stdio::from(std::io::stderr()))
        .status()
        .ok()
        .and_then(|status| status.code())
}

/// SY-09: the on-disk size of the task's folders under `root` (P-12).
fn size(id: TaskId, root: &Path) -> u64 {
    let mut seen = HashSet::new();
    id.folders()
        .iter()
        .map(|folder| root.join(folder.trim_start_matches('/')))
        .filter_map(|folder| {
            fs::symlink_metadata(&folder)
                .ok()
                .map(|meta| (folder, meta))
        })
        .map(|(folder, meta)| tally(&folder, meta.dev(), &mut seen))
        .sum()
}

/// Read-only: never follows symlinks or crosses filesystems; what can't be read counts 0.
fn tally(path: &Path, dev: u64, seen: &mut HashSet<(u64, u64)>) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.dev() != dev || !seen.insert((meta.dev(), meta.ino())) {
        return 0;
    }
    let own = meta.blocks() * 512;
    if !meta.is_dir() {
        return own;
    }
    let children = fs::read_dir(path).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| tally(&entry.path(), dev, seen))
            .sum()
    });
    own + children
}

impl fmt::Display for Preview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.tasks.is_empty() {
            writeln!(f, "No system tasks to run.")?;
        }
        for task in &self.tasks {
            writeln!(f, "  {:<42}{:>10}", task.name, human_size(task.bytes))?;
            for run in &task.previews {
                match (&run.exit, &run.summary) {
                    (Some(0), Some(summary)) => writeln!(f, "    preview: {summary}")?,
                    (Some(0), None) => writeln!(f, "    preview: nothing to remove")?,
                    _ => writeln!(f, "    preview `{}` failed", run.command)?,
                }
            }
            for command in &task.would_run {
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

impl fmt::Display for SystemResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for task in &self.tasks {
            write!(f, "  {:<42}{:>10}", task.name, human_size(task.freed))?;
            if let Some(failed) = task.commands.iter().find(|run| run.exit != Some(0)) {
                let exit = failed
                    .exit
                    .map_or("didn't run".to_owned(), |code| format!("exit {code}"));
                write!(f, "  `{}` failed ({exit})", failed.command)?;
            }
            writeln!(f)?;
        }
        writeln!(f, "Freed {}.", human_size(self.freed))
    }
}

//! `onyx system` (specs/007-system.md): the one privileged step (P-04). onyx runs owning
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

impl TaskId {
    /// The id `--only` and `--skip` take, and completions offer (CO-03).
    pub fn id(self) -> &'static str {
        match self {
            Self::PacmanCache => "pacman-cache",
            Self::Journal => "journal",
            Self::Coredumps => "coredumps",
        }
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
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
}

/// SY-03: what a read-only preview printed, stdout then stderr, without blank lines.
#[derive(Debug, Serialize)]
pub struct PreviewOutput {
    pub command: String,
    /// `None` when the command couldn't start or was killed by a signal.
    pub exit: Option<i32>,
    pub lines: Vec<String>,
}

/// SY-03: at most this many of a preview's lines are shown.
const SHOWN_LINES: usize = 10;

/// SY-12: what a task's live commands would remove, worked out before they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WouldFree {
    Exactly(u64),
    About(u64),
    NotKnown,
}

impl WouldFree {
    fn bytes(self) -> Option<u64> {
        match self {
            Self::Exactly(bytes) | Self::About(bytes) => Some(bytes),
            Self::NotKnown => None,
        }
    }
}

impl fmt::Display for WouldFree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exactly(bytes) => f.write_str(&human_size(*bytes)),
            Self::About(bytes) => write!(f, "about {}", human_size(*bytes)),
            Self::NotKnown => f.write_str("not known"),
        }
    }
}

/// SY-13: `"would_free"` as bytes or `null`, and `"estimate"`.
impl Serialize for WouldFree {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("would_free", &self.bytes())?;
        map.serialize_entry("estimate", &matches!(self, Self::About(_)))?;
        map.end()
    }
}

#[derive(Debug, Serialize)]
pub struct TaskPreview {
    pub id: TaskId,
    pub name: &'static str,
    /// SY-03: the size now.
    pub bytes: u64,
    #[serde(flatten)]
    pub would_free: WouldFree,
    pub previews: Vec<PreviewOutput>,
    /// The exact command lines a live run runs.
    pub would_run: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Preview {
    pub tasks: Vec<TaskPreview>,
    pub inactive: Vec<TaskId>,
}

/// SY-12: the journal is trimmed to this.
const JOURNAL_LIMIT: u64 = 500 * 1024 * 1024;

/// SY-12
fn would_free(id: TaskId, bytes: u64, previews: &[PreviewOutput], root: &Path) -> WouldFree {
    match id {
        TaskId::PacmanCache => previews
            .iter()
            .map(|run| run.lines.iter().find_map(|line| paccache_saved(line)))
            .sum::<Option<u64>>()
            .map_or(WouldFree::NotKnown, WouldFree::Exactly),
        TaskId::Journal => WouldFree::About(bytes.saturating_sub(JOURNAL_LIMIT)),
        TaskId::Coredumps => {
            let folder = root.join(id.folders()[0].trim_start_matches('/'));
            let empty = fs::read_dir(folder).map_or(true, |mut entries| entries.next().is_none());
            if empty {
                WouldFree::Exactly(0)
            } else {
                WouldFree::NotKnown
            }
        }
    }
}

/// SY-12: paccache's `disk space saved: 6.37 MiB`, or 0 for `no candidate packages`.
fn paccache_saved(line: &str) -> Option<u64> {
    if line.contains("no candidate packages found") {
        return Some(0);
    }
    let (_, after) = line.split_once("disk space saved: ")?;
    let mut words = after.trim_end_matches(')').split_whitespace();
    let number: f64 = words.next()?.parse().ok()?;
    let unit = match words.next()? {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    // A size is never negative, and the cast saturates at u64's range.
    Some((number * unit).round().max(0.0) as u64)
}

/// SY-03, SY-04: measures each task under `root` and runs its read-only previews.
pub fn preview(tasks: &Tasks, selected: &[Task], root: &Path, search_path: &[PathBuf]) -> Preview {
    let tasks_preview = selected
        .iter()
        .map(|task| {
            let bytes = size(task.id, root);
            let previews: Vec<PreviewOutput> = task
                .previews
                .iter()
                .map(|run| run_preview(run, search_path))
                .collect();
            TaskPreview {
                id: task.id,
                name: task.id.name(),
                bytes,
                would_free: would_free(task.id, bytes, &previews, root),
                previews,
                would_run: match &tasks.sudo {
                    Some(sudo) => task
                        .commands
                        .iter()
                        .map(|run| run.under(sudo).to_string())
                        .collect(),
                    None => Vec::new(),
                },
            }
        })
        .collect();
    Preview {
        tasks: tasks_preview,
        inactive: tasks.inactive.clone(),
    }
}

/// SY-04: no shell, no sudo, stdin closed; the output is kept for the preview only.
fn run_preview(run: &Invocation, search_path: &[PathBuf]) -> PreviewOutput {
    let output = run.command(search_path).stdin(Stdio::null()).output().ok();
    let lines = output.as_ref().map_or_else(Vec::new, |output| {
        [&output.stdout, &output.stderr]
            .into_iter()
            .flat_map(|bytes| {
                String::from_utf8_lossy(bytes)
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect()
    });
    PreviewOutput {
        command: run.to_string(),
        exit: output.as_ref().and_then(|output| output.status.code()),
        lines,
    }
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
            writeln!(
                f,
                "  {:<26}now {:>10}   would free {}",
                task.name,
                human_size(task.bytes),
                task.would_free
            )?;
            for run in &task.previews {
                write_preview(f, run)?;
            }
            for command in &task.would_run {
                writeln!(f, "    would run: {command}")?;
            }
        }
        if !self.tasks.is_empty() {
            writeln!(f, "{}", Total::of(&self.tasks))?;
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

/// SY-03: every line, up to `SHOWN_LINES`, then how many more.
fn write_preview(f: &mut fmt::Formatter<'_>, run: &PreviewOutput) -> fmt::Result {
    if run.exit != Some(0) {
        writeln!(f, "    preview `{}` failed", run.command)?;
    } else if run.lines.is_empty() {
        writeln!(f, "    preview: nothing to remove")?;
    }
    for line in run.lines.iter().take(SHOWN_LINES) {
        writeln!(f, "    preview: {line}")?;
    }
    let more = run.lines.len().saturating_sub(SHOWN_LINES);
    if more > 0 {
        writeln!(f, "    preview: … {more} more lines")?;
    }
    Ok(())
}

/// SY-13: the tasks' `would free` together.
struct Total {
    bytes: u64,
    estimate: bool,
    not_known: Vec<&'static str>,
}

impl Total {
    fn of(tasks: &[TaskPreview]) -> Self {
        Self {
            bytes: tasks
                .iter()
                .filter_map(|task| task.would_free.bytes())
                .sum(),
            estimate: tasks
                .iter()
                .any(|task| matches!(task.would_free, WouldFree::About(_))),
            not_known: tasks
                .iter()
                .filter(|task| task.would_free == WouldFree::NotKnown)
                .map(|task| task.name)
                .collect(),
        }
    }
}

impl fmt::Display for Total {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let about = if self.estimate { "about " } else { "" };
        write!(f, "Would free {about}{} in total", human_size(self.bytes))?;
        if !self.not_known.is_empty() {
            write!(f, " ({} not known)", self.not_known.join(", "))?;
        }
        write!(f, ".")
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

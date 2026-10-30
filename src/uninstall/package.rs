//! UN-08 to UN-15: the package that owns an app, removed only by its manager's own
//! command through `sudo` (P-04, P-07).

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

use super::AppName;
use crate::rules::find_tool;
use crate::safe_delete::CommandLog;
use crate::system::{CommandRun, Invocation, run_live};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Manager {
    Pacman,
    Apt,
    Dnf,
    Zypper,
    /// UN-16: installed for this user.
    FlatpakUser,
    FlatpakSystem,
    Snap,
}

impl Manager {
    /// UN-08, UN-16: the order they're tried in.
    const ALL: [Self; 7] = [
        Self::Pacman,
        Self::Apt,
        Self::Dnf,
        Self::Zypper,
        Self::FlatpakUser,
        Self::FlatpakSystem,
        Self::Snap,
    ];

    /// The tool that answers who owns what, and the one that removes.
    fn tools(self) -> (&'static str, &'static str) {
        match self {
            Self::Pacman => ("pacman", "pacman"),
            Self::Apt => ("dpkg-query", "apt-get"),
            Self::Dnf => ("rpm", "dnf"),
            Self::Zypper => ("rpm", "zypper"),
            Self::FlatpakUser | Self::FlatpakSystem => ("flatpak", "flatpak"),
            Self::Snap => ("snap", "snap"),
        }
    }

    /// UN-17: a user's own flatpak needs no root.
    fn needs_sudo(self) -> bool {
        self != Self::FlatpakUser
    }

    /// UN-16: how `query` tells whether `name` is installed, also used as its preview.
    fn installed(self) -> &'static [&'static str] {
        match self {
            Self::Pacman => &["-Qq"],
            Self::Apt => &["-W", "-f=${Status}"],
            Self::Dnf | Self::Zypper => &["-q"],
            Self::FlatpakUser => &["info", "--user"],
            Self::FlatpakSystem => &["info", "--system"],
            Self::Snap => &["list"],
        }
    }

    /// UN-08: the installed package named `name`, or else the one owning `program`.
    fn owner(self, query: &Path, name: &str, program: Option<&Path>) -> Option<String> {
        let args: Vec<&str> = self.installed().iter().copied().chain([name]).collect();
        let by_name = ask(query, &args)
            .filter(|answer| self != Self::Apt || answer == "install ok installed")
            .map(|_| name.to_owned());
        let by_program = || {
            let program = program?.to_str()?;
            match self {
                Self::Pacman => ask(query, &["-Qqo", program]),
                Self::Apt => dpkg_owner(&ask(query, &["-S", program])?).map(str::to_owned),
                Self::Dnf | Self::Zypper => ask(query, &["-qf", "--qf", "%{NAME}", program]),
                // UN-16: by the exact name only.
                Self::FlatpakUser | Self::FlatpakSystem | Self::Snap => None,
            }
        };
        by_name.or_else(by_program).filter(|package| plain(package))
    }

    /// UN-10: what removing `package` would do, found without root.
    fn preview(self, query: &Path, remover: &Path, package: &str) -> Invocation {
        let (program, args): (&Path, &[&str]) = match self {
            // `-n` and `--print` conflict; `-n` only changes saved config files.
            Self::Pacman => (remover, &["-Rs", "--print"]),
            Self::Apt => (remover, &["-s", "remove"]),
            Self::Dnf | Self::Zypper | Self::FlatpakUser | Self::FlatpakSystem | Self::Snap => {
                (query, self.installed())
            }
        };
        invocation(program, args, package)
    }

    /// UN-11: never `--noconfirm` or `-y`, so the manager asks for itself.
    fn removal(self, remover: &Path, package: &str) -> Invocation {
        let args: &[&str] = match self {
            Self::Pacman => &["-Rns"],
            Self::Apt | Self::Dnf | Self::Zypper | Self::Snap => &["remove"],
            // No `--delete-data`: molt trashes `~/.var/app/N` itself (UN-19).
            Self::FlatpakUser => &["uninstall", "--user"],
            Self::FlatpakSystem => &["uninstall", "--system"],
        };
        invocation(remover, args, package)
    }
}

impl fmt::Display for Manager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tools().1)
    }
}

fn invocation(program: &Path, args: &[&str], package: &str) -> Invocation {
    Invocation {
        program: program.to_owned(),
        args: args
            .iter()
            .copied()
            .chain([package])
            .map(str::to_owned)
            .collect(),
    }
}

/// UN-09: a query's trimmed output when it succeeds. No shell, no sudo, stdin closed;
/// `PATH` is already only the absolute entries the tool was found on.
fn ask(query: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new(query)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// `dpkg-query -S` prints `package: /path`, or `a, b: /path` when several share it.
fn dpkg_owner(output: &str) -> Option<&str> {
    output
        .lines()
        .next()?
        .split_once(": ")
        .map(|(owner, _)| owner)
}

/// A single package name that can't be read as an option or as several names.
fn plain(package: &str) -> bool {
    !package.is_empty()
        && !package.starts_with('-')
        && package
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+:@-".contains(&b))
}

#[derive(Debug)]
pub struct Package {
    pub manager: Manager,
    pub name: String,
    preview: Invocation,
    removal: Invocation,
}

impl Package {
    /// UN-11, UN-15, UN-17: the exact live command, or `None` when it needs a `sudo`
    /// that isn't there.
    pub fn command(&self, sudo: Option<&Path>) -> Option<Invocation> {
        if self.manager.needs_sudo() {
            sudo.map(|sudo| self.removal.under(sudo))
        } else {
            Some(self.removal.clone())
        }
    }
}

/// UN-08: the first manager on `search_path` that owns `app`.
pub fn find(app: &AppName, search_path: &[PathBuf]) -> Option<Package> {
    let name = app.to_string();
    let program = find_tool(&name, search_path);
    Manager::ALL.into_iter().find_map(|manager| {
        let (query, remover) = manager.tools();
        let query = find_tool(query, search_path)?;
        let remover = find_tool(remover, search_path)?;
        let package = manager.owner(&query, &name, program.as_deref())?;
        Some(Package {
            manager,
            preview: manager.preview(&query, &remover, &package),
            removal: manager.removal(&remover, &package),
            name: package,
        })
    })
}

#[derive(Debug, Serialize)]
pub struct PackagePreview {
    pub manager: Manager,
    pub name: String,
    pub preview: PreviewRun,
    /// UN-15: `None` when it needs a `sudo` that isn't there.
    pub would_run: Option<String>,
}

/// UN-10: runs the read-only preview and names the exact live command.
pub fn preview(package: &Package, sudo: Option<&Path>, search_path: &[PathBuf]) -> PackagePreview {
    PackagePreview {
        manager: package.manager,
        name: package.name.clone(),
        preview: run_full(&package.preview, search_path),
        would_run: package.command(sudo).map(|run| run.to_string()),
    }
}

/// A preview and everything it printed, as the list of what goes is the point of it.
#[derive(Debug, Serialize)]
pub struct PreviewRun {
    pub command: String,
    /// `None` when it couldn't start or was killed by a signal.
    pub exit: Option<i32>,
    pub lines: Vec<String>,
}

/// UN-09: no shell, no sudo, stdin closed; its stderr is passed through.
fn run_full(run: &Invocation, search_path: &[PathBuf]) -> PreviewRun {
    let output = run
        .command(search_path)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .ok();
    PreviewRun {
        command: run.to_string(),
        exit: output.as_ref().and_then(|output| output.status.code()),
        lines: output
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

impl fmt::Display for PackagePreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Package")?;
        writeln!(f, "  {} ({})", self.name, self.manager)?;
        if self.preview.exit == Some(0) {
            writeln!(f, "    preview:")?;
            for line in &self.preview.lines {
                writeln!(f, "      {line}")?;
            }
        } else {
            writeln!(f, "    preview `{}` failed", self.preview.command)?;
        }
        match &self.would_run {
            Some(command) => writeln!(f, "    would run: {command}")?,
            None => writeln!(
                f,
                "    left out: sudo isn't installed, so only the leftovers go"
            )?,
        }
        writeln!(f)
    }
}

/// UN-11, UN-13: runs the package's `command` and logs it (SY-11).
pub fn remove(
    run: &Invocation,
    search_path: &[PathBuf],
    log: &mut CommandLog,
) -> std::io::Result<CommandRun> {
    let exit = run_live(run, search_path);
    log.record(&run.words(), exit, 0)?;
    Ok(CommandRun {
        command: run.to_string(),
        exit,
        summary: None,
    })
}

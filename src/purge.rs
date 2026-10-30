//! `onyx purge` (specs/008-purge.md): build folders in your own projects. Finding only;
//! the preview, consent and deletion are `clean`'s (PG-07).

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use crate::rules::{Category, Findings, Kind, Match, Rule, RuleId, SkipReason, Skipped, find_tool};
use crate::safe_delete::{Candidate, Errno, Mode};

#[cfg(test)]
mod tests;

/// PG-04: a project changed more recently than this is being worked on.
const RECENT: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectKind {
    Cargo,
    Node,
    PythonVenv,
    Gradle,
    Maven,
    Next,
    Zig,
}

impl ProjectKind {
    pub const ALL: [Self; 7] = [
        Self::Cargo,
        Self::Node,
        Self::PythonVenv,
        Self::Gradle,
        Self::Maven,
        Self::Next,
        Self::Zig,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Node => "node",
            Self::PythonVenv => "python-venv",
            Self::Gradle => "gradle",
            Self::Maven => "maven",
            Self::Next => "next",
            Self::Zig => "zig",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Cargo => "Cargo target folders",
            Self::Node => "node_modules folders",
            Self::PythonVenv => "Python virtual environments",
            Self::Gradle => "Gradle build folders",
            Self::Maven => "Maven target folders",
            Self::Next => "Next.js build folders",
            Self::Zig => "Zig build folders",
        }
    }

    fn folders(self) -> &'static [&'static str] {
        match self {
            Self::Cargo | Self::Maven => &["target"],
            Self::Node => &["node_modules"],
            Self::PythonVenv => &[".venv", "venv"],
            Self::Gradle => &["build", ".gradle"],
            Self::Next => &[".next"],
            Self::Zig => &["zig-cache", ".zig-cache", "zig-out"],
        }
    }

    /// PG-02: whether `project` holds this kind's marker, and `folder` is what it builds.
    fn builds(self, project: &Path, folder: &Path) -> bool {
        let has = |name: &str| project.join(name).is_file();
        match self {
            Self::Cargo => has("Cargo.toml"),
            Self::Node => has("package.json"),
            Self::PythonVenv => {
                (has("pyproject.toml") || has("requirements.txt") || has("setup.py"))
                    && folder.join("pyvenv.cfg").is_file()
            }
            Self::Gradle => has("build.gradle") || has("build.gradle.kts"),
            Self::Maven => has("pom.xml"),
            Self::Next => has("package.json") && has_next_config(project),
            Self::Zig => has("build.zig"),
        }
    }
}

fn has_next_config(project: &Path) -> bool {
    fs::read_dir(project).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("next.config."))
        })
    })
}

/// One rule per kind, with no paths and no command: onyx deletes build folders itself
/// through safe delete, as `cargo clean` and the like would run project code (008).
pub fn rules() -> Vec<Rule> {
    ProjectKind::ALL
        .iter()
        .map(|kind| Rule {
            id: RuleId::builtin(kind.id()),
            name: kind.name().to_owned(),
            category: Category::DevTools,
            kind: Kind::Regenerable,
            paths: Vec::new(),
            clean_with: Vec::new(),
            processes: Vec::new(),
            requires: Vec::new(),
        })
        .collect()
}

/// PG-01 to PG-06: the build folders under `roots`. `search_path` is where `git` is
/// looked for; `now` decides what is recent.
pub fn find(roots: &[PathBuf], search_path: &[PathBuf], now: SystemTime) -> Findings {
    let mut finder = Finder {
        git: find_tool("git", search_path),
        search_path,
        now,
        findings: Findings::default(),
    };
    for root in roots {
        match fs::symlink_metadata(root) {
            Ok(meta) if meta.file_type().is_symlink() => {
                finder.skip(walk_id(), root.clone(), SkipReason::Symlink);
            }
            Ok(meta) if meta.is_dir() => finder.walk(root, meta.dev()),
            Ok(_) => {}
            Err(e) => finder.skip(walk_id(), root.clone(), unreadable(&e)),
        }
    }
    finder.findings
}

struct Finder<'a> {
    git: Option<PathBuf>,
    search_path: &'a [PathBuf],
    now: SystemTime,
    findings: Findings,
}

impl Finder<'_> {
    /// PG-03: never follows symlinks or crosses filesystems, and never enters a candidate
    /// or a hidden folder.
    fn walk(&mut self, dir: &Path, dev: u64) {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                return self.skip(walk_id(), dir.to_owned(), unreadable(&e));
            }
        };
        let subdirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect();
        let builds = self.builds_in(dir, &subdirs);
        for subdir in &subdirs {
            if let Some((_, kind)) = builds.iter().find(|(folder, _)| folder == subdir) {
                self.consider(dir, subdir, *kind, &builds);
                continue;
            }
            let hidden = subdir
                .file_name()
                .is_some_and(|name| name.as_encoded_bytes().starts_with(b"."));
            let same_fs = fs::symlink_metadata(subdir).is_ok_and(|meta| meta.dev() == dev);
            if !hidden && same_fs {
                self.walk(subdir, dev);
            }
        }
    }

    /// The build folders in `dir`, each claimed by the first kind that builds it (PG-06).
    fn builds_in(&self, dir: &Path, subdirs: &[PathBuf]) -> Vec<(PathBuf, ProjectKind)> {
        subdirs
            .iter()
            .filter_map(|subdir| {
                let name = subdir.file_name()?.to_str()?;
                ProjectKind::ALL
                    .into_iter()
                    .find(|kind| kind.folders().contains(&name) && kind.builds(dir, subdir))
                    .map(|kind| (subdir.clone(), kind))
            })
            .collect()
    }

    fn consider(
        &mut self,
        project: &Path,
        folder: &Path,
        kind: ProjectKind,
        builds: &[(PathBuf, ProjectKind)],
    ) {
        if self.changed_since(project, builds) {
            return self.skip(
                RuleId::builtin(kind.id()),
                folder.to_owned(),
                SkipReason::RecentlyChanged,
            );
        }
        if self.may_be_tracked(project, folder) {
            return self.skip(
                RuleId::builtin(kind.id()),
                folder.to_owned(),
                SkipReason::GitTracked,
            );
        }
        match Candidate::scan(folder) {
            Ok(candidate) => self.findings.matches.push(Match {
                rule: RuleId::builtin(kind.id()),
                candidate,
                mode: Mode::Permanent,
            }),
            Err(errno) => self.skip(
                RuleId::builtin(kind.id()),
                folder.to_owned(),
                SkipReason::Unreadable(errno),
            ),
        }
    }

    /// PG-04: anything directly in the project, or in its `src`, other than its build
    /// folders, changed within `RECENT` of now. Unknown counts as recent (P-02).
    fn changed_since(&self, project: &Path, builds: &[(PathBuf, ProjectKind)]) -> bool {
        let Some(cutoff) = self.now.checked_sub(RECENT) else {
            return true;
        };
        let newer = |dir: &Path| -> bool {
            fs::read_dir(dir).map_or(true, |entries| {
                entries.filter_map(Result::ok).any(|entry| {
                    let path = entry.path();
                    !builds.iter().any(|(folder, _)| *folder == path)
                        && fs::symlink_metadata(&path)
                            .and_then(|meta| meta.modified())
                            .map_or(true, |modified| modified > cutoff)
                })
            })
        };
        let src = project.join("src");
        newer(project) || (src.is_dir() && newer(&src))
    }

    /// PG-05: inside a Git repository, `git ls-files` must say the folder holds nothing
    /// tracked. No `git`, a failure, or any output means it may (P-02).
    fn may_be_tracked(&self, project: &Path, folder: &Path) -> bool {
        let in_repo = project
            .ancestors()
            .any(|dir| fs::symlink_metadata(dir.join(".git")).is_ok());
        if !in_repo {
            return false;
        }
        let (Some(git), Some(name)) = (&self.git, folder.file_name()) else {
            return true;
        };
        Command::new(git)
            // No fsmonitor hook: a repository's config must not make onyx run its code.
            .args(["-c", "core.fsmonitor=false", "ls-files", "-z", "--"])
            .arg(name)
            .current_dir(project)
            .env(
                "PATH",
                std::env::join_paths(self.search_path).unwrap_or_default(),
            )
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_or(true, |out| !out.status.success() || !out.stdout.is_empty())
    }

    fn skip(&mut self, rule: RuleId, path: PathBuf, reason: SkipReason) {
        self.findings.skipped.push(Skipped { rule, path, reason });
    }
}

/// Skips found while walking `purge_paths`, before any kind applies.
fn walk_id() -> RuleId {
    RuleId::builtin("purge-paths")
}

fn unreadable(e: &std::io::Error) -> SkipReason {
    SkipReason::Unreadable(Errno::from_io_error(e).unwrap_or(Errno::IO))
}

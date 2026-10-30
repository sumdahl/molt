//! Rules engine (specs/002-rules-engine.md): cleaning rules as data, matched to candidates.
//! It never deletes anything; it only finds candidates for safe delete.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::safe_delete::{Candidate, Errno, Mode};

#[cfg(test)]
mod tests;

/// A rule's id: lowercase ASCII letters, digits and `-` (RE-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct RuleId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    DevTools,
    Browsers,
    Apps,
    SystemUser,
    Trash,
}

/// What kind of data a rule finds. Decides the default mode (000 Decisions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Regenerable,
    UserContent,
}

/// The base directory a pattern starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Home,
    Config,
    Cache,
    Data,
    State,
}

/// A validated path pattern (RE-05, RE-06).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct Pattern {
    raw: String,
    anchor: Anchor,
    components: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: RuleId,
    pub name: String,
    pub category: Category,
    pub kind: Kind,
    pub paths: Vec<Pattern>,
    /// The owning tool's own cleanup command (P-07).
    #[serde(default)]
    pub clean_with: Vec<String>,
    /// Processes that own the data (P-08).
    #[serde(default)]
    pub processes: Vec<String>,
    /// Executables, any one of which on `$PATH` makes the rule apply (RE-08).
    #[serde(default)]
    pub requires: Vec<String>,
}

/// The directories anchors resolve to (RE-07). Passed in, never read from the process.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub home: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
}

#[derive(Debug)]
pub enum LoadError {
    /// RE-02: the user rules directory or a file in it exists but can't be read.
    Read { path: PathBuf, source: io::Error },
    /// RE-02, RE-03: a rule file isn't valid. `origin` names the file.
    Parse {
        origin: String,
        source: toml::de::Error,
    },
    /// RE-04.
    DuplicateId {
        id: RuleId,
        first: String,
        second: String,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(f, "can't read {}: {source}", path.display()),
            Self::Parse { origin, source } => write!(f, "{origin}: {source}"),
            Self::DuplicateId { id, first, second } => {
                write!(f, "rule id `{id}` is defined in both {first} and {second}")
            }
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::DuplicateId { .. } => None,
        }
    }
}

const BUILT_IN: &str = include_str!("../rules/builtin.toml");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    #[serde(default)]
    rule: Vec<Rule>,
}

/// Loads the built-in rules, then the user's from `user_dir` (RE-01).
pub fn load(user_dir: &Path) -> Result<Vec<Rule>, LoadError> {
    let mut rules = Vec::new();
    let mut origins = HashMap::new();
    let mut add = |origin: &str, text: &str| -> Result<(), LoadError> {
        for rule in parse(origin, text)? {
            if let Some(first) = origins.insert(rule.id.clone(), origin.to_owned()) {
                return Err(LoadError::DuplicateId {
                    id: rule.id,
                    first,
                    second: origin.to_owned(),
                });
            }
            rules.push(rule);
        }
        Ok(())
    };
    add("built-in rules", BUILT_IN)?;
    for path in user_rule_files(user_dir)? {
        let text = std::fs::read_to_string(&path).map_err(|source| LoadError::Read {
            path: path.clone(),
            source,
        })?;
        add(&path.display().to_string(), &text)?;
    }
    Ok(rules)
}

/// `*.toml` files in `dir`, in name order. A missing directory just means no user rules.
fn user_rule_files(dir: &Path) -> Result<Vec<PathBuf>, LoadError> {
    let read_error = |source| LoadError::Read {
        path: dir.to_path_buf(),
        source,
    };
    let entries = match std::fs::read_dir(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        other => other.map_err(read_error)?,
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry.map_err(read_error)?.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Parses one rule file. `origin` names it in errors.
pub fn parse(origin: &str, text: &str) -> Result<Vec<Rule>, LoadError> {
    let file: RuleFile = toml::from_str(text).map_err(|source| LoadError::Parse {
        origin: origin.to_owned(),
        source,
    })?;
    let mut seen = HashSet::new();
    if let Some(dup) = file.rule.iter().find(|rule| !seen.insert(&rule.id)) {
        return Err(LoadError::DuplicateId {
            id: dup.id.clone(),
            first: origin.to_owned(),
            second: origin.to_owned(),
        });
    }
    Ok(file.rule)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// RE-10.
    Symlink,
    /// RE-09: a directory on the way couldn't be listed.
    Unreadable(Errno),
}

#[derive(Debug)]
pub struct Match {
    pub rule: RuleId,
    pub candidate: Candidate,
    pub mode: Mode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub rule: RuleId,
    pub path: PathBuf,
    pub reason: SkipReason,
}

#[derive(Debug, Default)]
pub struct Findings {
    pub matches: Vec<Match>,
    pub skipped: Vec<Skipped>,
    /// RE-08: rules whose required tools aren't installed.
    pub inactive: Vec<RuleId>,
}

/// Matches every applicable rule against the filesystem. `search_path` is `$PATH`.
pub fn find(rules: &[Rule], dirs: &Dirs, search_path: &[PathBuf]) -> Findings {
    let mut findings = Findings::default();
    let mut seen = HashSet::new();
    for rule in rules {
        if !rule.applies(search_path) {
            findings.inactive.push(rule.id.clone());
            continue;
        }
        let mode = match rule.kind {
            Kind::Regenerable => Mode::Permanent,
            Kind::UserContent => Mode::Trash,
        };
        for pattern in &rule.paths {
            let mut skip = |path, reason| {
                findings.skipped.push(Skipped {
                    rule: rule.id.clone(),
                    path,
                    reason,
                })
            };
            for path in pattern.expand(dirs.resolve(pattern.anchor), &mut skip) {
                // RE-11: a path two rules match belongs to the first.
                if !seen.insert(path.clone()) {
                    continue;
                }
                match Candidate::scan(&path) {
                    Ok(candidate) => findings.matches.push(Match {
                        rule: rule.id.clone(),
                        candidate,
                        mode,
                    }),
                    // Gone or unreadable since it was listed.
                    Err(errno) => skip(path, SkipReason::Unreadable(errno)),
                }
            }
        }
    }
    findings
}

impl Rule {
    /// RE-08: no `requires`, or at least one of them is an executable on the search path.
    fn applies(&self, search_path: &[PathBuf]) -> bool {
        self.requires.is_empty()
            || self
                .requires
                .iter()
                .any(|tool| search_path.iter().any(|dir| is_executable(&dir.join(tool))))
    }
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

impl Dirs {
    fn resolve(&self, anchor: Anchor) -> &Path {
        match anchor {
            Anchor::Home => &self.home,
            Anchor::Config => &self.config,
            Anchor::Cache => &self.cache,
            Anchor::Data => &self.data,
            Anchor::State => &self.state,
        }
    }
}

impl Pattern {
    /// RE-09, RE-10: every existing path under `base` the pattern names, found by listing
    /// directories without following symlinks. Symlinks and unlistable directories are
    /// passed to `skip`.
    fn expand(&self, base: &Path, skip: &mut impl FnMut(PathBuf, SkipReason)) -> Vec<PathBuf> {
        let mut level = vec![base.to_path_buf()];
        for (i, component) in self.components.iter().enumerate() {
            let last = i + 1 == self.components.len();
            let mut next = Vec::new();
            for dir in &level {
                for path in children(dir, component, skip) {
                    match std::fs::symlink_metadata(&path) {
                        Ok(meta) if meta.file_type().is_symlink() => {
                            skip(path, SkipReason::Symlink)
                        }
                        Ok(meta) if last || meta.is_dir() => next.push(path),
                        _ => {}
                    }
                }
            }
            level = next;
        }
        level
    }
}

/// Entries of `dir` that `component` names, sorted. A literal names at most one entry.
fn children(
    dir: &Path,
    component: &str,
    skip: &mut impl FnMut(PathBuf, SkipReason),
) -> Vec<PathBuf> {
    if !is_glob(component) {
        return vec![dir.join(component)];
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            skip(dir.to_path_buf(), SkipReason::Unreadable(errno(&e)));
            return Vec::new();
        }
    };
    let mut names: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| {
            name.to_str()
                .is_some_and(|name| glob_match(component, name))
        })
        .collect();
    names.sort();
    names.into_iter().map(|name| dir.join(name)).collect()
}

fn errno(e: &io::Error) -> Errno {
    Errno::from_io_error(e).unwrap_or(Errno::IO)
}

fn is_glob(component: &str) -> bool {
    component.contains(['*', '?'])
}

/// `*` matches any run of characters (including a leading `.`), `?` exactly one.
fn glob_match(pattern: &str, name: &str) -> bool {
    let (pattern, name): (Vec<char>, Vec<char>) =
        (pattern.chars().collect(), name.chars().collect());
    let (mut p, mut n) = (0, 0);
    // Where the last `*` was, and how much of the name it has taken so far.
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((star_p, star_n)) => {
                    p = star_p + 1;
                    n = star_n + 1;
                    star = Some((star_p, star_n + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for RuleId {
    type Error = String;

    /// RE-03: lowercase ASCII letters, digits and `-`.
    fn try_from(id: String) -> Result<Self, String> {
        let valid = !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if valid {
            Ok(Self(id))
        } else {
            Err(format!("invalid rule id {id:?}: use a-z, 0-9 and -"))
        }
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl TryFrom<String> for Pattern {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, String> {
        let invalid = |why: &str| Err(format!("invalid path pattern {raw:?}: {why}"));
        let mut parts = raw.split('/');
        let anchor = match parts.next() {
            Some("~") => Anchor::Home,
            Some("$XDG_CONFIG_HOME") => Anchor::Config,
            Some("$XDG_CACHE_HOME") => Anchor::Cache,
            Some("$XDG_DATA_HOME") => Anchor::Data,
            Some("$XDG_STATE_HOME") => Anchor::State,
            _ => return invalid("must start with ~ or an $XDG_*_HOME anchor"),
        };
        let components: Vec<String> = parts.map(str::to_owned).collect();
        // RE-05
        if components.is_empty() {
            return invalid("names nothing below the anchor");
        }
        for component in &components {
            if component.is_empty() || component == "." || component == ".." {
                return invalid("empty, `.` and `..` components are not allowed");
            }
            if component.contains("**") {
                return invalid("`**` is not allowed");
            }
        }
        // RE-06: no rule may match everything in a base directory.
        if is_glob(&components[0]) {
            return invalid("the first component after the anchor must be literal");
        }
        Ok(Self {
            raw,
            anchor,
            components,
        })
    }
}

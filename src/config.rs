//! Config and whitelist (specs/003-config.md): `$XDG_CONFIG_HOME/molt/config.toml`.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::rules::{Rule, RuleId};
use crate::safe_delete::Execution;

#[cfg(test)]
mod tests;

/// A path the user wrote in the config: `~/…` or absolute, literal, no `.` or `..` (CF-04).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct ConfigPath {
    from_home: bool,
    components: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// CF-07: preview only, unless `--live`.
    pub dry_run: bool,
    pub whitelist: Vec<ConfigPath>,
    pub disabled_rules: Vec<RuleId>,
    pub purge_paths: Vec<ConfigPath>,
}

#[derive(Debug)]
pub enum ConfigError {
    /// CF-02: the file exists but can't be read.
    Read { path: PathBuf, source: io::Error },
    /// CF-02, CF-03, CF-04.
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    /// CF-06.
    UnknownRule(RuleId),
    /// CF-07: `MOLT_DRY_RUN` is set to something other than `0` or `1`.
    BadDryRunEnv(OsString),
}

/// What the command line asked for. `--dry-run` with `--live` is a usage error that the
/// argument parser rejects before this point (004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requested {
    Default,
    DryRun,
    Live,
}

/// Why a run is a dry run, so the output can say so (CF-07).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DryRunCause {
    Flag,
    Config,
    Environment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Live,
    DryRun(DryRunCause),
}

impl Decision {
    pub fn execution(self) -> Execution {
        match self {
            Self::Live => Execution::Live,
            Self::DryRun(_) => Execution::DryRun,
        }
    }
}

/// Reads the config at `path`. A missing file means the defaults (CF-01).
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
        other => other.map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?,
    };
    toml::from_str(&text).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

impl Config {
    /// CF-06: the loaded rules minus the disabled ones. Every disabled id must exist.
    pub fn enabled_rules(&self, rules: Vec<Rule>) -> Result<Vec<Rule>, ConfigError> {
        if let Some(unknown) = self
            .disabled_rules
            .iter()
            .find(|id| !rules.iter().any(|rule| rule.id == **id))
        {
            return Err(ConfigError::UnknownRule(unknown.clone()));
        }
        Ok(rules
            .into_iter()
            .filter(|rule| !self.disabled_rules.contains(&rule.id))
            .collect())
    }

    /// CF-07: whether this run changes anything. `env` is `MOLT_DRY_RUN`.
    pub fn decide(
        &self,
        requested: Requested,
        env: Option<&OsStr>,
    ) -> Result<Decision, ConfigError> {
        // P-03: a bad value is an error even when a flag would decide anyway.
        let env_dry_run = match env {
            None => false,
            Some(value) if value == "0" => false,
            Some(value) if value == "1" => true,
            Some(value) => return Err(ConfigError::BadDryRunEnv(value.to_owned())),
        };
        Ok(match requested {
            Requested::DryRun => Decision::DryRun(DryRunCause::Flag),
            Requested::Live => Decision::Live,
            Requested::Default if env_dry_run => Decision::DryRun(DryRunCause::Environment),
            Requested::Default if self.dry_run => Decision::DryRun(DryRunCause::Config),
            Requested::Default => Decision::Live,
        })
    }
}

impl ConfigPath {
    /// The absolute path, with `~` resolved from `home` (CF-04).
    pub fn resolve(&self, home: &Path) -> PathBuf {
        let base = if self.from_home { home } else { Path::new("/") };
        self.components
            .iter()
            .fold(base.to_path_buf(), |path, c| path.join(c))
    }
}

impl TryFrom<String> for ConfigPath {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, String> {
        let invalid = |why: &str| Err(format!("invalid path {raw:?}: {why}"));
        let (from_home, rest) = if raw == "~" {
            (true, "")
        } else if let Some(rest) = raw.strip_prefix("~/") {
            (true, rest)
        } else if let Some(rest) = raw.strip_prefix('/') {
            (false, rest)
        } else {
            return invalid("must start with ~/ or /");
        };
        let components: Vec<String> = match rest {
            "" => Vec::new(),
            rest => rest.split('/').map(str::to_owned).collect(),
        };
        if !from_home && components.is_empty() {
            return invalid("names the whole filesystem");
        }
        for component in &components {
            if component.is_empty() || component == "." || component == ".." {
                return invalid("empty, `.` and `..` components are not allowed");
            }
            if component.contains(['*', '?', '$']) {
                return invalid("globs and variables are not allowed; write the literal path");
            }
        }
        Ok(Self {
            from_home,
            components,
        })
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(f, "can't read {}: {source}", path.display()),
            Self::Parse { path, source } => write!(f, "{}: {source}", path.display()),
            Self::UnknownRule(id) => write!(f, "disabled_rules names unknown rule `{id}`"),
            Self::BadDryRunEnv(value) => write!(f, "MOLT_DRY_RUN must be 0 or 1, not {value:?}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::UnknownRule(_) | Self::BadDryRunEnv(_) => None,
        }
    }
}

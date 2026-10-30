//! Startup (specs/004-cli.md): everything that runs before a command, in order. Any
//! failure stops onyx before the command runs.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::config::{self, Config, ConfigError, Decision, DryRunCause, Requested};
use crate::rules::{self, Category, Dirs, LoadError, Rule};
use crate::safe_delete::{Context, Uid};

#[cfg(test)]
mod tests;

/// What the environment says about where things live (CL-04, CL-05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub dirs: Dirs,
    /// `None` when `ONYX_NO_OPLOG=1`.
    pub oplog: Option<PathBuf>,
    /// `ONYX_DRY_RUN`, checked by `Config::decide` (CF-07).
    pub dry_run: Option<OsString>,
    /// Absolute `$PATH` entries, where rules look for their tools (SC-10).
    pub search_path: Vec<PathBuf>,
    pub euid: Uid,
}

/// Everything a command needs, loaded and checked.
#[derive(Debug)]
pub struct Startup {
    pub env: Environment,
    pub config: Config,
    /// The rules `scan` and `clean` use.
    pub rules: Vec<Rule>,
    /// IN-01: the `installers` rules, which only `onyx installer` uses.
    pub installers: Vec<Rule>,
    pub decision: Decision,
}

#[derive(Debug)]
pub enum StartupError {
    /// CL-07.
    Root,
    /// CL-04: why `$HOME` can't be used.
    Home(&'static str),
    /// CL-05.
    BadNoOplog(OsString),
    /// CL-08.
    Config(ConfigError),
    /// CL-08.
    Rules(LoadError),
}

/// Reads the environment through `var`. `euid` is the effective uid.
pub fn environment(
    var: impl Fn(&str) -> Option<OsString>,
    euid: Uid,
) -> Result<Environment, StartupError> {
    // CL-07: before anything else, so root never reads root's files.
    if euid.is_root() {
        return Err(StartupError::Root);
    }
    let home = home(var("HOME"))?;
    let xdg = |name: &str, default: &str| {
        var(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(default))
    };
    let dirs = Dirs {
        config: xdg("XDG_CONFIG_HOME", ".config"),
        cache: xdg("XDG_CACHE_HOME", ".cache"),
        data: xdg("XDG_DATA_HOME", ".local/share"),
        state: xdg("XDG_STATE_HOME", ".local/state"),
        home,
    };
    let oplog = match var("ONYX_NO_OPLOG") {
        Some(value) if value == "1" => None,
        Some(value) if value != "0" => return Err(StartupError::BadNoOplog(value)),
        _ => Some(dirs.state.join("onyx/operations.log")),
    };
    Ok(Environment {
        dirs,
        oplog,
        dry_run: var("ONYX_DRY_RUN"),
        search_path: var("PATH")
            .map(|path| {
                std::env::split_paths(&path)
                    .filter(|dir| dir.is_absolute())
                    .collect()
            })
            .unwrap_or_default(),
        euid,
    })
}

/// CL-04: `$HOME` must be set, absolute and valid UTF-8.
fn home(value: Option<OsString>) -> Result<PathBuf, StartupError> {
    let value = value.ok_or(StartupError::Home("is not set"))?;
    let home = value
        .into_string()
        .map_err(|_| StartupError::Home("is not valid UTF-8"))?;
    if Path::new(&home).is_absolute() {
        Ok(PathBuf::from(home))
    } else {
        Err(StartupError::Home("is not an absolute path"))
    }
}

/// Runs startup steps 2 to 5 (004 Model) for the given command-line request.
pub fn start(
    requested: Requested,
    var: impl Fn(&str) -> Option<OsString>,
    euid: Uid,
) -> Result<Startup, StartupError> {
    let env = environment(var, euid)?;
    let onyx_config = env.dirs.config.join("onyx");
    let config = config::load(&onyx_config.join("config.toml"))?;
    let (installers, rules) = config
        .enabled_rules(rules::load(&onyx_config.join("rules"))?)?
        .into_iter()
        .partition(|rule| rule.category == Category::Installers);
    let decision = config.decide(requested, env.dry_run.as_deref())?;
    Ok(Startup {
        env,
        config,
        rules,
        installers,
        decision,
    })
}

impl Environment {
    /// The context safe delete is given, with the config's whitelist (CF-05).
    pub fn context(&self, config: &Config) -> Context {
        let dirs = &self.dirs;
        Context {
            home: dirs.home.clone(),
            data_home: dirs.data.clone(),
            xdg_dirs: vec![
                dirs.config.clone(),
                dirs.cache.clone(),
                dirs.data.clone(),
                dirs.state.clone(),
            ],
            whitelist: config
                .whitelist
                .iter()
                .map(|path| path.resolve(&dirs.home))
                .collect(),
            oplog: self.oplog.clone(),
            euid: self.euid,
        }
    }
}

/// CL-06: what a command that changes files says when the config or the environment made
/// its run a dry run. `None` when the user asked for it, or the run is live.
pub fn dry_run_notice(decision: Decision) -> Option<String> {
    let source = match decision {
        Decision::DryRun(DryRunCause::Environment) => "ONYX_DRY_RUN=1",
        Decision::DryRun(DryRunCause::Config) => "dry_run = true in the config",
        Decision::DryRun(DryRunCause::Flag) | Decision::Live => return None,
    };
    Some(format!("dry run because of {source}; pass --live to act"))
}

impl From<ConfigError> for StartupError {
    fn from(error: ConfigError) -> Self {
        Self::Config(error)
    }
}

impl From<LoadError> for StartupError {
    fn from(error: LoadError) -> Self {
        Self::Rules(error)
    }
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root => f.write_str("refusing to run as root; run onyx as your normal user"),
            Self::Home(why) => write!(f, "$HOME {why}"),
            Self::BadNoOplog(value) => write!(f, "ONYX_NO_OPLOG must be 0 or 1, not {value:?}"),
            Self::Config(error) => error.fmt(f),
            Self::Rules(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for StartupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(error) => Some(error),
            Self::Rules(error) => Some(error),
            Self::Root | Self::Home(_) | Self::BadNoOplog(_) => None,
        }
    }
}

//! Tests for specs/004-cli.md startup logic. The binary-level tests are in tests/smoke.rs.

use super::*;

use std::collections::HashMap;
use std::fs;

use tempfile::TempDir;

use crate::config::DryRunCause;

const USER: u32 = 1000;

/// A fake environment: only the variables given exist.
fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
        .collect();
    move |key| map.get(key).cloned()
}

fn env(pairs: &[(&str, &str)]) -> Result<Environment, StartupError> {
    environment(vars(pairs), Uid::from_raw(USER))
}

// Environment

#[test]
fn cl_04_home_must_be_set_absolute_and_utf8() {
    assert!(matches!(env(&[]), Err(StartupError::Home(_))));
    assert!(matches!(env(&[("HOME", "")]), Err(StartupError::Home(_))));
    assert!(matches!(
        env(&[("HOME", "relative/home")]),
        Err(StartupError::Home(_))
    ));
    let non_utf8 = move |key: &str| {
        (key == "HOME").then(|| {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(b"/home/caf\xe9".to_vec())
        })
    };
    assert!(matches!(
        environment(non_utf8, Uid::from_raw(USER)),
        Err(StartupError::Home(_))
    ));
}

#[test]
fn cl_04_xdg_dirs_are_used_only_when_absolute() {
    let defaults = env(&[("HOME", "/h")]).unwrap().dirs;
    assert_eq!(
        defaults,
        Dirs {
            home: "/h".into(),
            config: "/h/.config".into(),
            cache: "/h/.cache".into(),
            data: "/h/.local/share".into(),
            state: "/h/.local/state".into(),
        }
    );

    let custom = env(&[
        ("HOME", "/h"),
        ("XDG_CONFIG_HOME", "/x/config"),
        ("XDG_CACHE_HOME", "relative/cache"),
        ("XDG_DATA_HOME", ""),
        ("XDG_STATE_HOME", "/x/state"),
    ])
    .unwrap()
    .dirs;
    assert_eq!(custom.config, Path::new("/x/config"));
    assert_eq!(custom.cache, Path::new("/h/.cache"));
    assert_eq!(custom.data, Path::new("/h/.local/share"));
    assert_eq!(custom.state, Path::new("/x/state"));
}

#[test]
fn cl_05_the_oplog_lives_in_state_and_can_be_turned_off() {
    let log = Some(PathBuf::from("/h/.local/state/molt/operations.log"));
    assert_eq!(env(&[("HOME", "/h")]).unwrap().oplog, log);
    assert_eq!(
        env(&[("HOME", "/h"), ("MOLT_NO_OPLOG", "0")])
            .unwrap()
            .oplog,
        log
    );
    assert_eq!(
        env(&[("HOME", "/h"), ("MOLT_NO_OPLOG", "1")])
            .unwrap()
            .oplog,
        None
    );
    for bad in ["yes", "", "true"] {
        assert!(
            matches!(
                env(&[("HOME", "/h"), ("MOLT_NO_OPLOG", bad)]),
                Err(StartupError::BadNoOplog(_))
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn cl_05_the_context_carries_dirs_oplog_whitelist_and_uid() {
    let env = env(&[("HOME", "/h"), ("XDG_CACHE_HOME", "/c")]).unwrap();
    let config: Config = toml::from_str("whitelist = [\"~/keep\"]").unwrap();

    let ctx = env.context(&config);

    assert_eq!(ctx.home, Path::new("/h"));
    assert_eq!(ctx.data_home, Path::new("/h/.local/share"));
    assert!(ctx.xdg_dirs.contains(&PathBuf::from("/c")));
    assert_eq!(ctx.whitelist, [PathBuf::from("/h/keep")]);
    assert_eq!(ctx.oplog, env.oplog);
    assert_eq!(ctx.euid, Uid::from_raw(USER));
}

// Root

#[test]
fn cl_07_root_is_refused_before_anything_else() {
    // Even an environment that is otherwise broken reports root first.
    assert!(matches!(
        environment(vars(&[]), Uid::ROOT),
        Err(StartupError::Root)
    ));
    assert!(matches!(
        start(Requested::Default, vars(&[("HOME", "/h")]), Uid::ROOT),
        Err(StartupError::Root)
    ));
}

// Failing closed

/// A fake home for `start`, with its config dir.
struct Home {
    _dir: TempDir,
    home: String,
    molt_config: PathBuf,
}

impl Home {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let home = dir.path().to_str().unwrap().to_owned();
        let molt_config = dir.path().join(".config/molt");
        fs::create_dir_all(molt_config.join("rules")).unwrap();
        Self {
            _dir: dir,
            home,
            molt_config,
        }
    }

    fn start(&self, extra: &[(&str, &str)]) -> Result<Startup, StartupError> {
        let mut pairs = vec![("HOME", self.home.as_str())];
        pairs.extend_from_slice(extra);
        start(Requested::Default, vars(&pairs), Uid::from_raw(USER))
    }
}

#[test]
fn cl_08_startup_loads_config_and_rules_and_decides() {
    let home = Home::new();
    fs::write(
        home.molt_config.join("config.toml"),
        "dry_run = true\ndisabled_rules = [\"thumbnails\"]\n",
    )
    .unwrap();

    let startup = home.start(&[]).unwrap();

    assert!(startup.config.dry_run);
    assert!(!startup.rules.is_empty());
    assert!(
        startup
            .rules
            .iter()
            .all(|r| r.id.to_string() != "thumbnails")
    );
    assert_eq!(startup.decision, Decision::DryRun(DryRunCause::Config));
}

#[test]
fn cl_08_bad_config_rules_or_dry_run_value_stop_startup() {
    let home = Home::new();
    let config = home.molt_config.join("config.toml");

    fs::write(&config, "colour = \"red\"").unwrap();
    assert!(matches!(home.start(&[]), Err(StartupError::Config(_))));

    fs::write(&config, "disabled_rules = [\"no-such-rule\"]").unwrap();
    assert!(matches!(home.start(&[]), Err(StartupError::Config(_))));

    fs::write(&config, "").unwrap();
    fs::write(
        home.molt_config.join("rules/bad.toml"),
        "[[rule]]\nid = 1\n",
    )
    .unwrap();
    assert!(matches!(home.start(&[]), Err(StartupError::Rules(_))));

    fs::write(home.molt_config.join("rules/bad.toml"), "").unwrap();
    assert!(matches!(
        home.start(&[("MOLT_DRY_RUN", "yes")]),
        Err(StartupError::Config(ConfigError::BadDryRunEnv(_)))
    ));
    assert!(home.start(&[]).is_ok());
}

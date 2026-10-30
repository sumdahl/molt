//! Tests for specs/003-config.md. Each test names the requirement it proves.
//! Every filesystem test uses a temporary directory as the home.

use super::*;

use std::fs;

use tempfile::TempDir;

use crate::rules;
use crate::safe_delete::{self, Candidate, Context, Mode, Outcome, Plan, Privilege, Reason, Uid};

fn parse(text: &str) -> Result<Config, ConfigError> {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(&path, text).unwrap();
    load(&path)
}

fn built_in_rules() -> Vec<Rule> {
    rules::load(Path::new("/nonexistent")).unwrap()
}

// Loading

#[test]
fn cf_01_a_missing_config_means_the_defaults() {
    let dir = TempDir::new().unwrap();

    let config = load(&dir.path().join("config.toml")).unwrap();

    assert_eq!(config, Config::default());
    assert!(!config.dry_run);
}

#[test]
fn cf_02_an_unreadable_config_fails_closed() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    // A directory can't be read as a file, even by root.
    fs::create_dir(&path).unwrap();

    assert!(matches!(load(&path), Err(ConfigError::Read { .. })));
}

#[test]
fn cf_02_a_parse_error_names_the_file_and_line() {
    let err = parse("dry_run = true\nwhitelist = [\n")
        .unwrap_err()
        .to_string();

    assert!(err.contains("config.toml"), "{err}");
    assert!(err.contains("line 2"), "{err}");
}

#[test]
fn cf_03_unknown_keys_and_wrong_types_are_errors() {
    for text in [
        "colour = \"red\"",
        "dry_run = \"yes\"",
        "whitelist = \"~/a\"",
        "disabled_rules = [1]",
    ] {
        assert!(
            matches!(parse(text), Err(ConfigError::Parse { .. })),
            "{text}"
        );
    }
    let full = "dry_run = true\nwhitelist = [\"~/a\"]\ndisabled_rules = [\"thumbnails\"]\npurge_paths = [\"/data/src\"]\n";
    assert!(parse(full).is_ok());
}

// Values

#[test]
fn cf_04_config_paths_are_home_relative_or_absolute_and_literal() {
    for bad in [
        "relative/path",
        "~user/x",
        "$HOME/x",
        "~/$XDG_CACHE_HOME",
        "~/a/../b",
        "~/./a",
        "~//a",
        "~/a/",
        "~/a/*",
        "/data/?",
        "",
        "/",
    ] {
        let text = format!("whitelist = [{bad:?}]");
        assert!(parse(&text).is_err(), "{bad}");
        let text = format!("purge_paths = [{bad:?}]");
        assert!(parse(&text).is_err(), "{bad}");
    }

    let config = parse("whitelist = [\"~/work/keep\", \"/data/vms\", \"~\"]").unwrap();
    let home = Path::new("/fake/home");
    let resolved: Vec<PathBuf> = config.whitelist.iter().map(|p| p.resolve(home)).collect();
    assert_eq!(
        resolved,
        [
            PathBuf::from("/fake/home/work/keep"),
            PathBuf::from("/data/vms"),
            PathBuf::from("/fake/home"),
        ]
    );
}

#[test]
fn cf_05_whitelisted_paths_are_refused_by_safe_delete() {
    let dir = TempDir::new().unwrap();
    let home = fs::canonicalize(dir.path()).unwrap();
    let keep = home.join("work/keep/target");
    fs::create_dir_all(&keep).unwrap();
    let config = parse("whitelist = [\"~/work/keep\"]").unwrap();
    let ctx = Context {
        data_home: home.join(".local/share"),
        xdg_dirs: Vec::new(),
        whitelist: config.whitelist.iter().map(|p| p.resolve(&home)).collect(),
        oplog: None,
        home: home.clone(),
        euid: Uid::from_raw(1000),
    };
    let plan = Plan {
        candidates: vec![Candidate::scan(&keep).unwrap()],
        mode: Mode::Permanent,
        execution: Execution::DryRun,
        privilege: Privilege::User,
    };

    let report = safe_delete::execute(&plan, &ctx).unwrap();

    assert_eq!(report.results[0].1, Outcome::Skipped(Reason::Whitelisted));
}

#[test]
fn cf_06_disabled_rules_are_removed_and_must_exist() {
    let config = parse("disabled_rules = [\"thumbnails\"]").unwrap();
    let all = built_in_rules();

    let enabled = config.enabled_rules(all.clone()).unwrap();

    assert_eq!(enabled.len(), all.len() - 1);
    assert!(enabled.iter().all(|r| r.id.to_string() != "thumbnails"));

    let typo = parse("disabled_rules = [\"thumbnial\"]").unwrap();
    let err = typo.enabled_rules(all).unwrap_err();
    assert!(matches!(err, ConfigError::UnknownRule(ref id) if id.to_string() == "thumbnial"));
}

// Dry run by default

#[test]
fn cf_07_dry_run_by_default_from_config_or_environment() {
    use DryRunCause as C;
    use Requested as R;
    let off = Config::default();
    let on = Config {
        dry_run: true,
        ..Config::default()
    };
    let one = Some(OsStr::new("1"));
    let zero = Some(OsStr::new("0"));
    let cases = [
        (&off, R::Default, None, Decision::Live),
        (&off, R::Default, zero, Decision::Live),
        (&off, R::Default, one, Decision::DryRun(C::Environment)),
        (&on, R::Default, None, Decision::DryRun(C::Config)),
        (&on, R::Default, zero, Decision::DryRun(C::Config)),
        (&on, R::Live, one, Decision::Live),
        (&off, R::Live, None, Decision::Live),
        (&off, R::DryRun, None, Decision::DryRun(C::Flag)),
        (&on, R::DryRun, one, Decision::DryRun(C::Flag)),
    ];
    for (config, requested, env, expected) in cases {
        assert_eq!(
            config.decide(requested, env).unwrap(),
            expected,
            "dry_run={} {requested:?} {env:?}",
            config.dry_run
        );
    }
    assert_eq!(Decision::Live.execution(), Execution::Live);
    assert_eq!(Decision::DryRun(C::Config).execution(), Execution::DryRun);
}

#[test]
fn cf_07_a_bad_molt_dry_run_value_is_an_error() {
    for value in ["yes", "true", "", "2"] {
        for requested in [Requested::Default, Requested::Live, Requested::DryRun] {
            let result = Config::default().decide(requested, Some(OsStr::new(value)));
            assert!(
                matches!(result, Err(ConfigError::BadDryRunEnv(_))),
                "{value:?} {requested:?}"
            );
        }
    }
}

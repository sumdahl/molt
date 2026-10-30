//! Tests for specs/002-rules-engine.md. Each test names the requirement it proves.
//! Every filesystem test uses a temporary directory as the home.

use super::*;

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

use tempfile::TempDir;

/// A fake home with XDG dirs inside it, next to an `outside` dir.
struct Sandbox {
    _dir: TempDir,
    dirs: Dirs,
    outside: PathBuf,
    bin: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let home = root.join("home");
        let dirs = Dirs {
            config: home.join(".config"),
            cache: home.join(".cache"),
            data: home.join(".local/share"),
            state: home.join(".local/state"),
            home,
        };
        for d in [&dirs.config, &dirs.cache, &dirs.data, &dirs.state] {
            fs::create_dir_all(d).unwrap();
        }
        let outside = root.join("outside");
        let bin = root.join("bin");
        fs::create_dir_all(outside.join("app")).unwrap();
        fs::write(outside.join("app/junk"), "precious").unwrap();
        fs::create_dir(&bin).unwrap();
        Self {
            _dir: dir,
            dirs,
            outside,
            bin,
        }
    }

    fn file(&self, path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "junk").unwrap();
    }

    fn find(&self, toml: &str) -> Findings {
        find(
            &parse("test.toml", toml).unwrap(),
            &self.dirs,
            std::slice::from_ref(&self.bin),
        )
    }
}

fn rule(id: &str, paths: &str) -> String {
    format!(
        r#"
[[rule]]
id = "{id}"
name = "test rule"
category = "dev-tools"
kind = "regenerable"
paths = [{paths}]
"#
    )
}

fn ids(rules: &[Rule]) -> Vec<&str> {
    rules.iter().map(|r| r.id.0.as_str()).collect()
}

fn matched(findings: &Findings) -> Vec<&Path> {
    findings
        .matches
        .iter()
        .map(|m| m.candidate.path())
        .collect()
}

/// Permissions aren't enforced for root, so permission tests can't run as root.
fn is_root() -> bool {
    let probe = TempDir::new().unwrap();
    fs::metadata(probe.path()).unwrap().uid() == 0
}

// Loading

#[test]
fn re_01_built_ins_load_first_then_user_files_in_name_order() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("b.toml"), rule("user-b", r#""~/.b""#)).unwrap();
    fs::write(dir.path().join("a.toml"), rule("user-a", r#""~/.a""#)).unwrap();
    fs::write(dir.path().join("notes.txt"), "not a rule file").unwrap();

    let rules = load(dir.path()).unwrap();
    let built_in = load(&dir.path().join("missing")).unwrap();

    assert!(!built_in.is_empty());
    assert_eq!(&ids(&rules)[..built_in.len()], ids(&built_in));
    assert_eq!(&ids(&rules)[built_in.len()..], ["user-a", "user-b"]);
}

#[test]
fn re_02_an_unreadable_user_rule_file_fails_closed() {
    let dir = TempDir::new().unwrap();
    // A directory named like a rule file can't be read as one, even by root.
    fs::create_dir(dir.path().join("broken.toml")).unwrap();

    let err = load(dir.path()).unwrap_err();

    assert!(matches!(err, LoadError::Read { ref path, .. } if path.ends_with("broken.toml")));
}

#[test]
fn re_02_a_parse_error_names_the_file_and_line() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("mine.toml"), "[[rule]]\nid = \n").unwrap();

    let err = load(dir.path()).unwrap_err().to_string();

    assert!(err.contains("mine.toml"), "{err}");
    assert!(err.contains("line 2"), "{err}");
}

#[test]
fn re_03_unknown_missing_and_invalid_fields_are_errors() {
    let valid = rule("ok", r#""~/.ok""#);
    let broken = [
        valid.replace("name = ", "colour = \"red\"\nname = "),
        valid.replace("name = \"test rule\"\n", ""),
        valid.replace("regenerable", "disposable"),
        valid.replace("dev-tools", "games"),
        valid.replace("id = \"ok\"", "id = \"Not_OK\""),
        valid.replace("id = \"ok\"", "id = \"\""),
    ];

    assert!(parse("t", &valid).is_ok());
    for text in &broken {
        assert!(
            matches!(parse("t", text), Err(LoadError::Parse { .. })),
            "{text}"
        );
    }
}

#[test]
fn re_04_duplicate_ids_are_errors_within_and_across_files() {
    let twice = rule("dup", r#""~/.a""#) + &rule("dup", r#""~/.b""#);
    assert!(matches!(
        parse("t", &twice),
        Err(LoadError::DuplicateId { .. })
    ));

    let built_in = load(Path::new("/nonexistent")).unwrap();
    let dir = TempDir::new().unwrap();
    let taken = built_in[0].id.0.clone();
    fs::write(dir.path().join("x.toml"), rule(&taken, r#""~/.x""#)).unwrap();
    let err = load(dir.path()).unwrap_err();
    assert!(matches!(err, LoadError::DuplicateId { ref id, .. } if id.0 == taken));
}

// Path patterns

#[test]
fn re_05_patterns_must_start_with_an_anchor_and_stay_clean() {
    let bad = [
        "/etc/x",
        "relative/x",
        "$HOME/x",
        "~",
        "~/",
        "~/../x",
        "~/./x",
        "~//x",
        "~/a/**",
        "~/a/**/b",
    ];
    for pattern in bad {
        let text = rule("p", &format!("{pattern:?}"));
        assert!(parse("t", &text).is_err(), "{pattern}");
    }
    for pattern in [
        "~/.npm/_cacache",
        "$XDG_CACHE_HOME/a/*/b",
        "$XDG_STATE_HOME/x",
    ] {
        let text = rule("p", &format!("{pattern:?}"));
        assert!(parse("t", &text).is_ok(), "{pattern}");
    }
}

#[test]
fn re_06_the_first_component_after_the_anchor_is_literal() {
    for pattern in ["~/*", "~/.c*", "$XDG_CACHE_HOME/?", "$XDG_CONFIG_HOME/*/x"] {
        let text = rule("p", &format!("{pattern:?}"));
        assert!(parse("t", &text).is_err(), "{pattern}");
    }
}

#[test]
fn re_07_anchors_resolve_from_the_given_dirs() {
    let sb = Sandbox::new();
    let paths = [
        sb.dirs.home.join(".npm/_cacache"),
        sb.dirs.cache.join("c"),
        sb.dirs.config.join("cf"),
        sb.dirs.data.join("d"),
        sb.dirs.state.join("s"),
    ];
    paths.iter().for_each(|p| sb.file(p));
    let patterns = r#""~/.npm/_cacache", "$XDG_CACHE_HOME/c", "$XDG_CONFIG_HOME/cf", "$XDG_DATA_HOME/d", "$XDG_STATE_HOME/s""#;

    let findings = sb.find(&rule("all", patterns));

    assert_eq!(
        matched(&findings),
        paths.iter().map(PathBuf::as_path).collect::<Vec<_>>()
    );
}

// Applicability

#[test]
fn re_08_rules_apply_only_when_a_required_tool_is_on_the_path() {
    let sb = Sandbox::new();
    sb.file(&sb.dirs.cache.join("tool/x"));
    let text =
        rule("needs-tool", r#""$XDG_CACHE_HOME/tool""#) + "requires = [\"faketool\", \"other\"]\n";
    let free = rule("no-needs", r#""$XDG_CACHE_HOME/tool""#);

    let inactive = sb.find(&text);
    fs::write(sb.bin.join("faketool"), "").unwrap();
    let not_executable = sb.find(&text);
    fs::set_permissions(sb.bin.join("faketool"), fs::Permissions::from_mode(0o755)).unwrap();
    let active = sb.find(&text);

    assert_eq!(
        inactive
            .inactive
            .iter()
            .map(|id| id.0.as_str())
            .collect::<Vec<_>>(),
        ["needs-tool"]
    );
    assert!(inactive.matches.is_empty());
    assert_eq!(not_executable.inactive.len(), 1);
    assert!(active.inactive.is_empty());
    assert_eq!(active.matches.len(), 1);
    assert!(sb.find(&free).inactive.is_empty());
}

// Matching

#[test]
fn re_09_globs_expand_within_one_component_including_dotfiles() {
    let sb = Sandbox::new();
    let c = &sb.dirs.cache;
    for p in ["app/a/junk", "app/.hidden/junk", "app/b/keep", "app/c/junk"] {
        sb.file(&c.join(p));
    }

    let findings = sb.find(&rule(
        "g",
        r#""$XDG_CACHE_HOME/app/*/junk", "$XDG_CACHE_HOME/app/?/keep""#,
    ));

    assert_eq!(
        matched(&findings),
        [".hidden/junk", "a/junk", "c/junk", "b/keep"].map(|p| c.join("app").join(p))
    );
    assert!(findings.skipped.is_empty());
}

#[test]
fn re_09_a_pattern_that_matches_nothing_is_not_an_error() {
    let sb = Sandbox::new();
    let findings = sb.find(&rule(
        "none",
        r#""~/.nothing/here", "$XDG_CACHE_HOME/nope/*""#,
    ));
    assert!(findings.matches.is_empty() && findings.skipped.is_empty());
}

#[test]
fn re_09_an_unlistable_directory_is_reported() {
    if is_root() {
        eprintln!("skipped: permissions are not enforced for root");
        return;
    }
    let sb = Sandbox::new();
    let locked = sb.dirs.cache.join("locked");
    sb.file(&locked.join("x/junk"));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

    let findings = sb.find(&rule("l", r#""$XDG_CACHE_HOME/locked/*""#));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(findings.matches.is_empty());
    assert_eq!(
        findings.skipped[0].reason,
        SkipReason::Unreadable(Errno::ACCESS)
    );
}

#[test]
fn re_10_symlinks_are_skipped_as_matches_and_on_the_way() {
    let sb = Sandbox::new();
    let cache = &sb.dirs.cache;
    fs::create_dir_all(cache.join("app")).unwrap();
    symlink(&sb.outside, cache.join("app/linked")).unwrap();
    symlink(sb.outside.join("app"), cache.join("via")).unwrap();

    let findings = sb.find(&rule(
        "s",
        r#""$XDG_CACHE_HOME/app/*", "$XDG_CACHE_HOME/via/junk""#,
    ));

    assert!(findings.matches.is_empty(), "{:?}", matched(&findings));
    let skipped: Vec<_> = findings
        .skipped
        .iter()
        .map(|s| (s.path.clone(), s.reason))
        .collect();
    assert_eq!(
        skipped,
        [
            (cache.join("app/linked"), SkipReason::Symlink),
            (cache.join("via"), SkipReason::Symlink),
        ]
    );
}

#[test]
fn re_11_matches_carry_their_kinds_mode_and_are_reported_once() {
    let sb = Sandbox::new();
    let shared = sb.dirs.home.join("Downloads/setup.iso");
    sb.file(&shared);
    let user = rule("installers", r#""~/Downloads/*.iso""#).replace("regenerable", "user-content");
    let regen = rule("also", r#""~/Downloads/setup.iso""#);

    let findings = sb.find(&(user + &regen));

    assert_eq!(findings.matches.len(), 1);
    let m = &findings.matches[0];
    assert_eq!((m.rule.0.as_str(), m.mode), ("installers", Mode::Trash));
    assert_eq!(m.candidate.path(), shared);
    let regen_only = sb.find(&rule("r", r#""~/Downloads/setup.iso""#));
    assert_eq!(regen_only.matches[0].mode, Mode::Permanent);
}

#[test]
fn re_09_glob_matching_edge_cases() {
    let cases = [
        ("*.iso", "setup.iso", true),
        ("*.iso", "setup.iso.part", false),
        ("*", ".hidden", true),
        ("a*b*c", "aXbYbZc", true),
        ("a*b*c", "aXbYc_", false),
        ("?", "é", true),
        ("??", "a", false),
        ("*x", "", false),
        ("**", "anything", true),
    ];
    for (pattern, name, expected) in cases {
        assert_eq!(glob_match(pattern, name), expected, "{pattern} vs {name:?}");
    }
}

// Built-in rules (specs/010-built-in-rules.md)

/// For each built-in rule: paths under home it must match, and siblings it must not.
const BUILT_IN_LAYOUT: &[(&str, &[&str], &[&str])] = &[
    (
        "installer-deb",
        &["Downloads/a.deb", "Desktop/b.deb"],
        &[
            "Downloads/a.deb.part",
            "Documents/c.deb",
            "Downloads/sub/d.deb",
        ],
    ),
    (
        "installer-rpm",
        &["Downloads/a.rpm"],
        &["Downloads/a.rpmnew"],
    ),
    (
        "installer-pacman",
        &["Downloads/a-1-x86_64.pkg.tar.zst", "Desktop/b.pkg.tar.xz"],
        &["Downloads/c.tar.zst"],
    ),
    (
        "installer-appimage",
        &["Downloads/A.AppImage", "Desktop/b.appimage"],
        &["Downloads/A.AppImage.zsync"],
    ),
    (
        "installer-iso",
        &["Downloads/os.iso"],
        &["Downloads/disk.img"],
    ),
    ("installer-run", &["Desktop/driver.run"], &["Downloads/run"]),
    ("npm-cache", &[".npm/_cacache"], &[".npm/_logs"]),
    (
        "pnpm-store",
        &[".local/share/pnpm/store"],
        &[".local/share/pnpm/global"],
    ),
    ("yarn-cache", &[".cache/yarn"], &[".config/yarn"]),
    ("bun-cache", &[".bun/install/cache"], &[".bun/bin"]),
    (
        "cargo-registry",
        &[".cargo/registry/cache", ".cargo/registry/src"],
        &[".cargo/registry/index", ".cargo/bin"],
    ),
    ("cargo-git", &[".cargo/git/checkouts"], &[".cargo/git/db"]),
    ("go-build-cache", &[".cache/go-build"], &[".config/go"]),
    ("go-module-cache", &["go/pkg/mod"], &["go/bin"]),
    (
        "gradle-caches",
        &[".gradle/caches", ".gradle/wrapper/dists"],
        &[".gradle/gradle.properties"],
    ),
    (
        "maven-repository",
        &[".m2/repository"],
        &[".m2/settings.xml"],
    ),
    ("uv-cache", &[".cache/uv"], &[".local/share/uv"]),
    ("pip-cache", &[".cache/pip"], &[".config/pip"]),
    ("mise-cache", &[".cache/mise"], &[".local/share/mise"]),
    ("yay-cache", &[".cache/yay"], &[".config/yay"]),
    ("paru-cache", &[".cache/paru/clone"], &[".cache/paru/diff"]),
    (
        "jetbrains-cache",
        &[
            ".cache/JetBrains/IntelliJIdea2025.2",
            ".cache/JetBrains/RustRover2025.1",
        ],
        &[".config/JetBrains/IntelliJIdea2025.2"],
    ),
    (
        "firefox-cache",
        &[".cache/mozilla/firefox/abc.default/cache2"],
        &[
            ".cache/mozilla/firefox/abc.default/startupCache",
            ".mozilla/firefox/abc.default/places.sqlite",
        ],
    ),
    (
        "chrome-cache",
        &[
            ".cache/google-chrome/Default/Cache",
            ".cache/google-chrome/Profile 1/Code Cache",
        ],
        &[".config/google-chrome/Default/Login Data"],
    ),
    (
        "chromium-cache",
        &[
            ".cache/chromium/Default/Cache",
            ".cache/chromium/Default/Code Cache",
        ],
        &[".config/chromium/Default/Bookmarks"],
    ),
    (
        "brave-cache",
        &[
            ".cache/BraveSoftware/Brave-Browser/Default/Cache",
            ".cache/BraveSoftware/Brave-Browser/Default/Code Cache",
        ],
        &[".config/BraveSoftware/Brave-Browser/Default/Cookies"],
    ),
    (
        "spotify-cache",
        &[".cache/spotify"],
        &[".config/spotify/prefs"],
    ),
    (
        "discord-cache",
        &[".config/discord/Cache", ".config/discord/Code Cache"],
        &[
            ".config/discord/Local Storage",
            ".config/discord/settings.json",
        ],
    ),
    (
        "slack-cache",
        &[".config/Slack/Cache", ".config/Slack/Code Cache"],
        &[".config/Slack/storage"],
    ),
    (
        "thumbnails",
        &[".cache/thumbnails"],
        &[".local/share/recently-used.xbel"],
    ),
];

#[test]
fn br_01_br_05_each_built_in_rule_matches_exactly_its_cache() {
    let built_in = load(Path::new("/nonexistent")).unwrap();
    let mut tested: Vec<&str> = BUILT_IN_LAYOUT.iter().map(|(id, ..)| *id).collect();
    let mut shipped = ids(&built_in);
    tested.sort_unstable();
    shipped.sort_unstable();
    assert_eq!(tested, shipped, "every built-in rule needs a layout here");

    let sb = Sandbox::new();
    for tool in built_in.iter().flat_map(|r| &r.requires) {
        let exe = sb.bin.join(tool);
        fs::write(&exe, "").unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    }
    for (_, matching, other) in BUILT_IN_LAYOUT {
        for rel in matching.iter().chain(*other) {
            sb.file(&sb.dirs.home.join(rel).join("entry"));
        }
    }

    let findings = find(&built_in, &sb.dirs, std::slice::from_ref(&sb.bin));

    assert!(findings.inactive.is_empty(), "{:?}", findings.inactive);
    assert!(findings.skipped.is_empty(), "{:?}", findings.skipped);
    for (id, matching, _) in BUILT_IN_LAYOUT {
        let mut got: Vec<&Path> = findings
            .matches
            .iter()
            .filter(|m| m.rule.0 == *id)
            .map(|m| m.candidate.path())
            .collect();
        let mut want: Vec<PathBuf> = matching.iter().map(|rel| sb.dirs.home.join(rel)).collect();
        got.sort_unstable();
        want.sort_unstable();
        assert_eq!(got, want, "{id}");
    }
}

#[test]
fn br_02_rules_name_the_processes_that_own_their_data() {
    for rule in load(Path::new("/nonexistent")).unwrap() {
        // Thumbnails are written by many programs, and a missing one is just redrawn.
        // Downloaded installers aren't data any program keeps open (009).
        if rule.id.0 != "thumbnails" && rule.category != Category::Installers {
            assert!(!rule.processes.is_empty(), "{}", rule.id);
        }
    }
}

#[test]
fn br_03_cleanup_commands_are_exactly_the_reviewed_ones() {
    let expected = [
        ("npm-cache", "npm cache clean --force"),
        ("pnpm-store", "pnpm store prune"),
        ("yarn-cache", "yarn cache clean"),
        ("go-build-cache", "go clean -cache"),
        ("go-module-cache", "go clean -modcache"),
        ("uv-cache", "uv cache clean"),
        ("mise-cache", "mise cache clear"),
    ];
    let mut got: Vec<(String, String)> = load(Path::new("/nonexistent"))
        .unwrap()
        .into_iter()
        .filter(|r| !r.clean_with.is_empty())
        .map(|r| (r.id.0, r.clean_with.join(" ")))
        .collect();
    got.sort();
    let mut want: Vec<(String, String)> = expected
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect();
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn br_04_a_rule_with_a_cleanup_command_requires_that_tool() {
    for rule in load(Path::new("/nonexistent")).unwrap() {
        if let Some(tool) = rule.clean_with.first() {
            assert!(rule.requires.contains(tool), "{}", rule.id);
        }
    }
}

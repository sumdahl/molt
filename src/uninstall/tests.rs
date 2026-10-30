//! Tests for app names and the rules built from them (UN-01 to UN-03, UN-05).

use super::*;

#[test]
fn un_01_only_plain_names_are_accepted() {
    for good in [
        "foo",
        "org.gnome.Foo",
        "code-oss",
        "g++",
        "Foo_2",
        &"a".repeat(100),
    ] {
        assert!(AppName::try_from(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        ".foo",
        "-foo",
        "..",
        "a/b",
        "a b",
        "f*",
        "f?",
        "f[",
        "é",
        &"a".repeat(101),
    ] {
        assert_eq!(AppName::try_from(bad), Err(NameError::Invalid), "{bad:?}");
    }
}

#[test]
fn un_03_shared_folder_names_are_refused() {
    for shared in [
        "applications",
        "autostart",
        "Trash",
        "systemd",
        "molt",
        "local",
    ] {
        assert_eq!(
            AppName::try_from(shared),
            Err(NameError::Shared),
            "{shared}"
        );
    }
}

#[test]
fn un_02_un_05_one_rule_per_location_with_its_kind() {
    let rules = rules(&AppName::try_from("foo").unwrap()).unwrap();
    let got: Vec<(String, String, Kind)> = rules
        .iter()
        .map(|rule| (rule.id.to_string(), rule.paths[0].to_string(), rule.kind))
        .collect();
    let want = [
        (
            "uninstall-config",
            "$XDG_CONFIG_HOME/foo",
            Kind::UserContent,
        ),
        ("uninstall-data", "$XDG_DATA_HOME/foo", Kind::UserContent),
        ("uninstall-state", "$XDG_STATE_HOME/foo", Kind::UserContent),
        ("uninstall-cache", "$XDG_CACHE_HOME/foo", Kind::Regenerable),
        ("uninstall-dotfolder", "~/.foo", Kind::UserContent),
        (
            "uninstall-menu-entry",
            "$XDG_DATA_HOME/applications/foo.desktop",
            Kind::UserContent,
        ),
        (
            "uninstall-autostart",
            "$XDG_CONFIG_HOME/autostart/foo.desktop",
            Kind::UserContent,
        ),
    ];
    let want: Vec<_> = want
        .iter()
        .map(|(id, path, kind)| (id.to_string(), path.to_string(), *kind))
        .collect();
    assert_eq!(got, want);
    // P-08
    assert!(rules.iter().all(|rule| rule.processes == ["foo"]));
    assert!(rules.iter().all(|rule| rule.paths.len() == 1));
    // RuleId::builtin's ids must satisfy RE-03.
    assert!(
        rules
            .iter()
            .all(|rule| RuleId::try_from(rule.id.to_string()).is_ok())
    );
}

//! Tests for reading package managers' answers (UN-08).

use super::*;

#[test]
fn un_08_only_single_plain_package_names_are_used() {
    for good in [
        "foo",
        "foo-bin",
        "g++",
        "libfoo1:amd64",
        "python3.12",
        "foo@2",
    ] {
        assert!(plain(good), "{good}");
    }
    for bad in [
        "",
        "-Rdd",
        "--noconfirm",
        "a b",
        "a, b",
        "a\nb",
        "a;b",
        "a/b",
    ] {
        assert!(!plain(bad), "{bad:?}");
    }
}

#[test]
fn un_08_dpkg_names_the_owner_before_the_colon() {
    assert_eq!(dpkg_owner("foo: /usr/bin/foo\n"), Some("foo"));
    assert_eq!(
        dpkg_owner("libfoo1:amd64: /usr/bin/foo"),
        Some("libfoo1:amd64")
    );
    // Several owners stay one string, which `plain` then refuses.
    assert_eq!(dpkg_owner("a, b: /usr/bin/foo"), Some("a, b"));
    assert_eq!(dpkg_owner("no owner"), None);
}

#[test]
fn un_11_the_removal_never_skips_the_managers_own_question() {
    for manager in Manager::ALL {
        let run = manager.removal(Path::new("/usr/bin/tool"), "foo");
        // UN-18: snap has no question of its own; snapd keeps a snapshot instead.
        assert!(
            run.args
                .iter()
                .all(|arg| arg != "-y" && arg != "--noconfirm"),
            "{run}"
        );
        assert_eq!(run.args.last().map(String::as_str), Some("foo"));
    }
}

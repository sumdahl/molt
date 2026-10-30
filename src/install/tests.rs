//! Tests for the update's checks that don't need a release (SE-01, SE-02, SE-07).

use super::*;

#[test]
fn se_01_versions_compare_as_numbers() {
    assert!(is_newer("v0.2.0", "0.1.0"));
    assert!(is_newer("v0.10.0", "0.9.9"));
    assert!(!is_newer("v0.1.0", "0.1.0"));
    assert!(!is_newer("v0.0.9", "0.1.0"));
    for bad in ["0.2.0", "v0.2", "v0.2.0.1", "v0.2.x", "latest"] {
        assert!(!is_newer(bad, "0.1.0"), "{bad}");
    }
}

#[test]
fn se_02_the_built_in_signers_name_the_maintainer_for_releases() {
    assert_eq!(principals(SIGNERS), ["sumdahl"]);
    assert!(
        SIGNERS.contains("onyx-release"),
        "the key may sign releases"
    );
    assert_eq!(principals("# c\n\na x y\nb z w\n"), ["a", "b"]);
}

#[test]
fn se_02_the_checksum_must_name_exactly_this_file() {
    let hash = "a".repeat(64);
    let tarball = "onyx-v1.0.0-x86_64-unknown-linux-musl.tar.gz";
    assert!(names_only(&format!("{hash}  {tarball}\n"), tarball));
    assert!(names_only(&format!("{hash} *{tarball}\n"), tarball));
    assert!(!names_only(&format!("{hash}  other.tar.gz\n"), tarball));
    assert!(!names_only(
        &format!("{hash}  {tarball}\n{hash}  x\n"),
        tarball
    ));
    assert!(!names_only(&format!("abc  {tarball}\n"), tarball));
}

#[test]
fn se_02_only_plain_files_inside_the_top_folder_pass() {
    let top = "onyx-v1.0.0-x86_64-unknown-linux-musl";
    let names = format!("{top}/\n{top}/onyx\n{top}/README.md\n");
    let long = "drwxr-xr-x a/b 0 x\n-rwxr-xr-x a/b 9 x\n-rw-r--r-- a/b 1 x\n";
    assert!(entries_are_safe(&names, long, top));
    let link = long.replace("-rw-r--r--", "lrwxrwxrwx");
    assert!(!entries_are_safe(&names, &link, top), "a symlink");
    let escape = names.replace("README.md", "../.bashrc");
    assert!(!entries_are_safe(&escape, long, top));
    let outside = names.replace(&format!("{top}/README.md"), "elsewhere/README.md");
    assert!(!entries_are_safe(&outside, long, top));
    assert!(!entries_are_safe("", "", top), "an empty archive");
}

fn env_in(home: &Path, search_path: Vec<PathBuf>) -> Environment {
    let var = |name: &str| match name {
        "HOME" => Some(home.as_os_str().to_owned()),
        _ => None,
    };
    let euid = rustix::process::geteuid();
    Environment {
        search_path,
        ..crate::startup::environment(var, euid).unwrap()
    }
}

#[test]
fn pk_02_a_cargo_installed_onyx_is_cargos() {
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join(".cargo/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("onyx"), "").unwrap();
    let why = judge(bin.join("onyx"), &env_in(home.path(), vec![])).unwrap_err();
    assert!(why.contains("cargo install onyx-cli"), "{why}");
    assert!(why.contains("cargo uninstall onyx-cli"), "{why}");
}

#[test]
fn pk_02_a_packaged_onyx_names_its_package_and_manager() {
    let home = tempfile::tempdir().unwrap();
    let exe = PathBuf::from("/usr/bin/onyx");
    let cases = [
        (
            "pacman",
            r#"[ "$1 $2" = "-Qqo /usr/bin/onyx" ] && echo onyx-bin"#,
            "the onyx-bin package installed onyx at /usr/bin/onyx: update and remove it with pacman",
        ),
        (
            "dpkg-query",
            r#"[ "$1 $2" = "-S /usr/bin/onyx" ] && echo 'onyx: /usr/bin/onyx'"#,
            "the onyx package installed onyx at /usr/bin/onyx: update and remove it with apt-get",
        ),
    ];
    for (tool, script, said) in cases {
        let tools = tempfile::tempdir().unwrap();
        let path = tools.path().join(tool);
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        // Only the fake on the path, so the machine's own manager isn't asked.
        let env = env_in(home.path(), vec![tools.path().to_owned()]);
        assert_eq!(judge(exe.clone(), &env).unwrap_err(), said);
    }
}

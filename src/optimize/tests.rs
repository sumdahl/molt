//! Tests for reading each manager's list of packages nothing needs (OP-03 to OP-06).

use super::*;

#[test]
fn op_03_each_query_output_is_read_into_names() {
    assert_eq!(TaskId::PacmanOrphans.packages("a\nb-lib\n"), ["a", "b-lib"]);
    let apt = "NOTE: This is only a simulation!\nRemv libfoo1 [1.0-1]\nRemv bar [2]\nInst x\n";
    assert_eq!(TaskId::AptAutoremove.packages(apt), ["libfoo1", "bar"]);
    // dnf 4 puts each on a line; dnf 5 doesn't.
    assert_eq!(TaskId::DnfAutoremove.packages("a \nb \n"), ["a", "b"]);
    assert_eq!(TaskId::DnfAutoremove.packages("a b "), ["a", "b"]);
    let zypper = "S  | Repository | Name | Version | Arch\n\
                  ---+------------+------+---------+-----\n\
                  i  | repo-oss   | foo  | 1.0     | x86_64\n\
                  i  | repo-up    | foo  | 1.1     | x86_64\n\
                  i  | repo-oss   | bar  | 2       | noarch\n";
    assert_eq!(TaskId::ZypperUnneeded.packages(zypper), ["foo", "bar"]);
}

#[test]
fn op_04_a_name_that_could_be_an_option_or_several_leaves_the_task_out() {
    assert_eq!(
        found(TaskId::PacmanOrphans, Some(0), "a\n-Rdd\n"),
        Found::Unsafe("-Rdd".to_owned())
    );
    assert_eq!(
        found(TaskId::PacmanOrphans, Some(0), "a\n$(x)\n"),
        Found::Unsafe("$(x)".to_owned())
    );
}

#[test]
fn op_05_pacman_exit_1_with_no_output_is_nothing_and_other_failures_are_failures() {
    assert_eq!(found(TaskId::PacmanOrphans, Some(1), ""), Found::Nothing);
    assert_eq!(
        found(TaskId::PacmanOrphans, Some(1), "a\n"),
        Found::Failed(Some(1))
    );
    assert_eq!(
        found(TaskId::DnfAutoremove, Some(1), ""),
        Found::Failed(Some(1))
    );
    assert_eq!(
        found(TaskId::AptAutoremove, Some(0), "Reading...\n"),
        Found::Nothing
    );
    assert_eq!(found(TaskId::AptAutoremove, None, ""), Found::Failed(None));
}

#[test]
fn op_06_the_live_command_names_the_listed_packages_and_never_skips_the_question() {
    let tool = Path::new("/usr/bin/tool");
    let names = ["a".to_owned(), "b".to_owned()];
    let live = |id: TaskId| id.live(tool, &names).args;
    assert_eq!(live(TaskId::PacmanOrphans), ["-Rns", "a", "b"]);
    assert_eq!(live(TaskId::AptAutoremove), ["autoremove"]);
    assert_eq!(live(TaskId::DnfAutoremove), ["remove", "a", "b"]);
    assert_eq!(
        live(TaskId::ZypperUnneeded),
        ["remove", "--clean-deps", "a", "b"]
    );
    for id in TaskId::ALL {
        assert!(
            !live(id)
                .iter()
                .any(|arg| arg == "-y" || arg == "--noconfirm"),
            "{id}"
        );
    }
}

#[test]
fn op_10_op_11_op_13_maintenance_commands_are_exact_and_font_cache_needs_no_sudo() {
    let tool = Path::new("/usr/bin/tool");
    let live = |id: TaskId| id.live(tool, &[]).args;
    assert_eq!(live(TaskId::Fstrim), ["--all", "--verbose"]);
    assert!(live(TaskId::FontCache).is_empty());
    assert_eq!(live(TaskId::DnsCache), ["flush-caches"]);
    for id in [TaskId::Fstrim, TaskId::FontCache, TaskId::DnsCache] {
        assert!(
            !live(id).iter().any(|arg| arg == "-f" || arg == "--force"),
            "{id}"
        );
    }
    assert!(!TaskId::FontCache.needs_sudo());
    assert!(TaskId::ALL.iter().filter(|id| !id.needs_sudo()).count() == 1);
}

#[test]
fn op_12_the_dns_preview_is_the_cache_size_line_and_a_failure_leaves_it_out() {
    let stats = "Cache\n                         Current Cache Size:  43\n    Cache Hits: 242\n";
    assert_eq!(
        found(TaskId::DnsCache, Some(0), stats),
        Found::Ready(Some("Current Cache Size: 43".to_owned()))
    );
    assert_eq!(found(TaskId::DnsCache, Some(1), ""), Found::Failed(Some(1)));
}

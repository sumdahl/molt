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
    // OP-15: the checks need none either.
    let without: Vec<TaskId> = TaskId::ALL
        .into_iter()
        .filter(|id| !id.needs_sudo())
        .collect();
    assert_eq!(
        without,
        [TaskId::FontCache, TaskId::FailedUnits, TaskId::Smart]
    );
}

#[test]
fn op_12_the_dns_preview_is_the_cache_size_line_and_a_failure_leaves_it_out() {
    let stats = "Cache\n                         Current Cache Size:  43\n    Cache Hits: 242\n";
    assert_eq!(
        found(TaskId::DnsCache, Some(0), stats),
        Found::Ready(Some("43 answers cached".to_owned()))
    );
    assert_eq!(found(TaskId::DnsCache, Some(1), ""), Found::Failed(Some(1)));
}

#[test]
fn op_17_only_real_whole_disks_with_plain_names_are_checked() {
    let sys_block = tempfile::tempdir().unwrap();
    for name in [
        "sda", "nvme0n1", "loop0", "ram1", "zram0", "dm-0", "md127", "sr0", "fd0", "nbd3", "x y",
    ] {
        std::fs::create_dir(sys_block.path().join(name)).unwrap();
    }
    assert_eq!(disks(sys_block.path()), ["nvme0n1", "sda"]);
    assert!(disks(&sys_block.path().join("missing")).is_empty());
}

#[test]
fn op_18_each_smart_outcome_has_its_line() {
    let ata = "SMART overall-health self-assessment test result: PASSED\n";
    assert_eq!(smart_health("sda", Some(0), ata), "sda: passed");
    let scsi = "SMART Health Status: OK\n";
    assert_eq!(smart_health("sdb", Some(0), scsi), "sdb: passed");
    let failed = "SMART overall-health self-assessment test result: FAILED!\n";
    assert_eq!(smart_health("sda", Some(8), failed), "sda: FAILING");
    assert_eq!(smart_health("sda", Some(8), ""), "sda: FAILING");
    assert_eq!(
        smart_health("nvme0n1", Some(2), "Permission denied"),
        "nvme0n1: needs root: run `sudo smartctl -H /dev/nvme0n1`"
    );
    assert_eq!(
        smart_health("sdc", Some(4), "no data"),
        "sdc: no SMART data"
    );
}

#[test]
fn op_16_failed_units_are_named_with_where_to_look() {
    let system = Some((
        Some(0),
        "bad.service loaded failed failed Bad thing\n".to_owned(),
    ));
    let user = Some((Some(0), "mine.timer loaded failed failed Mine\n".to_owned()));
    let lines = failed_unit_lines(system, user);
    assert_eq!(lines[0], "bad.service");
    assert_eq!(lines[1], "mine.timer (user)");
    assert!(lines[2].contains("systemctl status <unit>"), "{lines:?}");
    assert!(lines[2].contains("journalctl -b -u <unit>"), "{lines:?}");

    let none = failed_unit_lines(
        Some((Some(0), String::new())),
        Some((Some(0), String::new())),
    );
    assert_eq!(none, ["no failed units"]);

    let broken = failed_unit_lines(
        Some((Some(1), String::new())),
        Some((Some(0), String::new())),
    );
    assert!(broken[0].contains("failed (exit 1)"), "{broken:?}");
}

#[test]
fn op_09_the_question_names_the_tasks_that_use_sudo() {
    let task = |id: TaskId| TaskPreview {
        id,
        name: id.name(),
        query: None,
        found: Found::Ready(None),
        would_run: Some(String::new()),
        live: Some(Invocation {
            program: "/bin/true".into(),
            args: Vec::new(),
        }),
    };
    let ask = |ids: &[TaskId]| {
        Preview {
            tasks: ids.iter().copied().map(task).collect(),
            inactive: Vec::new(),
        }
        .question()
    };
    assert_eq!(ask(&[TaskId::FontCache]), "Run all of the above?");
    assert_eq!(
        ask(&[TaskId::Fstrim, TaskId::FontCache]),
        "Run all of the above? fstrim uses sudo."
    );
    assert_eq!(
        ask(&[TaskId::Fstrim, TaskId::FontCache, TaskId::DnsCache]),
        "Run all of the above? fstrim, dns-cache use sudo."
    );
}

#[test]
fn op_12_one_dns_answer_is_singular() {
    assert_eq!(
        cache_size("  Current Cache Size:  1\n").as_deref(),
        Some("1 answer cached")
    );
    assert_eq!(cache_size("  Current Cache Size: lots\n"), None);
}

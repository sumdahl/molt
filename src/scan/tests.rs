//! Unit tests for specs/005-scan.md. The command itself is tested in tests/smoke.rs.

use super::*;

#[test]
fn sc_04_sizes_use_binary_units_with_one_decimal() {
    let cases = [
        (0, "0 B"),
        (512, "512 B"),
        (1023, "1023 B"),
        (1024, "1.0 KiB"),
        (1536, "1.5 KiB"),
        (1024 * 1024 - 1, "1.0 MiB"),
        (25_125_558_681, "23.4 GiB"),
        (5 * 1024u64.pow(4), "5.0 TiB"),
        (u64::MAX, "16.0 EiB"),
    ];
    for (bytes, expected) in cases {
        assert_eq!(human_size(bytes), expected, "{bytes}");
    }
}

#[test]
fn sc_02_failed_items_are_counted_on_the_rule_line() {
    let report = ScanReport {
        total: 0,
        partial: true,
        rules: vec![RuleReport {
            id: RuleId::try_from("x".to_owned()).unwrap(),
            name: "X cache".to_owned(),
            category: Category::Apps,
            mode: Mode::Permanent,
            bytes: 0,
            clean_with: Vec::new(),
            items: vec![Item {
                path: "/h/x".to_owned(),
                bytes: 0,
                outcome: "failed",
                reason: Some("Permission denied".to_owned()),
                partial: false,
            }],
            locked: Vec::new(),
        }],
        skipped: Vec::new(),
        inactive: Vec::new(),
        freed_by: Some("onyx clean"),
    };

    let text = report.to_string();

    assert!(
        text.contains("X cache") && text.contains("(1 failed)"),
        "{text}"
    );
    assert!(text.contains("Partial"), "{text}");
}

#[test]
fn sc_05_lf_06_one_is_singular_and_only_scan_names_a_command() {
    let report = |inactive: usize, freed_by| {
        ScanReport {
            total: 0,
            partial: false,
            rules: Vec::new(),
            skipped: Vec::new(),
            inactive: (0..inactive)
                .map(|i| RuleId::try_from(format!("r{i}")).unwrap())
                .collect(),
            freed_by,
        }
        .to_string()
    };
    let one = report(1, Some("onyx clean"));
    assert!(
        one.contains("1 rule inactive: the tool it cleans up after isn't installed."),
        "{one}"
    );
    assert!(
        one.contains("Total: 0 B can be freed with `onyx clean`."),
        "{one}"
    );
    let two = report(2, None);
    assert!(two.contains("2 rules inactive: the tools they"), "{two}");
    assert!(two.contains("Total: 0 B.\n"), "{two}");
    assert!(!two.contains("can be freed with"), "{two}");
}

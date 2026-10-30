//! Tests for reading and showing the operation log (HI-02 to HI-07).

use super::*;

const TRASHED: &str = r#"{"time":"2026-10-30T08:00:00Z","mode":"trash","path":"/h/a.deb","bytes":4096,"outcome":"trashed"}"#;
const REMOVED: &str = r#"{"time":"2026-10-30T09:00:00Z","mode":"permanent","path":"/h/.cache/x","bytes":2048,"outcome":"removed"}"#;
const FAILED: &str = r#"{"time":"2026-10-30T09:30:00Z","mode":"permanent","path":"/h/y","bytes":10,"outcome":"failed","partial":true,"errno":13}"#;
const RAN: &str = r#"{"time":"2026-10-30T10:00:00Z","command":["/usr/bin/sudo","--","/usr/bin/paccache","-rk2"],"exit":0,"bytes":1048576}"#;
const RAN_BADLY: &str = r#"{"time":"2026-10-30T11:00:00Z","command":["/usr/bin/sudo","--","/usr/bin/fstrim","--all"],"exit":null,"bytes":0}"#;

fn history(lines: &[&str]) -> History {
    parse(&lines.join("\n"))
}

#[test]
fn hi_06_bad_lines_are_skipped_and_counted() {
    let h = history(&[
        TRASHED,
        "not json",
        r#"{"time":"x","command":[],"exit":0,"bytes":0}"#,
        "",
        RAN,
    ]);
    assert_eq!(h.entries.len(), 2);
    assert_eq!(
        h.skipped_lines, 2,
        "an empty line isn't an entry, nor a bad one"
    );
    let unknown = TRASHED.replace("trashed", "eaten");
    assert_eq!(history(&[&unknown]).skipped_lines, 1);
}

#[test]
fn hi_02_each_entry_has_its_line() {
    let utc = TimeZone::UTC;
    let h = history(&[TRASHED, REMOVED, FAILED, RAN, RAN_BADLY]);
    let lines: Vec<String> = h.entries.iter().map(|entry| entry.line(&utc)).collect();
    assert_eq!(
        lines[0],
        "2026-10-30 08:00  trashed           4.0 KiB  /h/a.deb"
    );
    assert_eq!(
        lines[1],
        "2026-10-30 09:00  deleted           2.0 KiB  /h/.cache/x"
    );
    assert_eq!(
        lines[2],
        "2026-10-30 09:30  partly deleted       10 B  /h/y"
    );
    assert_eq!(
        lines[3],
        "2026-10-30 10:00  ran               1.0 MiB  /usr/bin/sudo -- /usr/bin/paccache -rk2"
    );
    assert_eq!(
        lines[4],
        "2026-10-30 11:00  ran                        /usr/bin/sudo -- /usr/bin/fstrim --all  (didn't run)"
    );
}

#[test]
fn hi_03_hi_04_hi_05_newest_first_limited_since_and_totalled() {
    let h = history(&[TRASHED, REMOVED, RAN]);
    let all = h.clone().select(None, None);
    let times: Vec<&str> = all
        .entries
        .iter()
        .map(|e| e.raw["time"].as_str().unwrap())
        .collect();
    assert_eq!(
        times,
        [
            "2026-10-30T10:00:00Z",
            "2026-10-30T09:00:00Z",
            "2026-10-30T08:00:00Z"
        ]
    );
    assert_eq!(all.freed(), 4096 + 2048 + 1048576);
    assert_eq!(all.trashed(), 1);

    let one = h.clone().select(None, Some(1));
    assert_eq!(one.entries.len(), 1);
    assert_eq!(one.left_out, 2);

    let since: Timestamp = "2026-10-30T08:30:00Z".parse().unwrap();
    assert_eq!(h.select(Some(since), None).entries.len(), 2);
}

#[test]
fn fp_06_confirmations_are_neither_shown_nor_unreadable() {
    let history = parse(concat!(
        r#"{"time":"2026-10-31T10:00:00Z","confirmed":"fingerprint","question":"Clean all of the above?"}"#,
        "\n",
        r#"{"time":"2026-10-31T10:00:01Z","mode":"trash","path":"/h/a","bytes":5,"outcome":"trashed"}"#,
        "\n",
    ));
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.skipped_lines, 0);
}

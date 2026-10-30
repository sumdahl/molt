//! Tests for the browser's keys, navigation and screen (AN-12 to AN-16), without a
//! terminal.

use super::*;

fn folder_node(name: &str, bytes: u64, children: Vec<Node>) -> Node {
    let mut node = Node::new(name.to_owned(), EntryKind::Dir, bytes);
    node.children = children;
    node
}

fn file(name: &str, bytes: u64) -> Node {
    Node::new(name.to_owned(), EntryKind::File, bytes)
}

/// /r (1000): big/ (600, holding inner/ 500), small/ (100), and a file of 300.
fn browser() -> Browser {
    let tree = folder_node(
        "/r",
        1000,
        vec![
            folder_node("big", 600, vec![folder_node("inner", 500, Vec::new())]),
            folder_node("small", 100, Vec::new()),
        ],
    );
    let mut browser = Browser::new(PathBuf::from("/r"), tree, vec![file("notes", 300)]);
    browser.set_rows(10);
    browser
}

fn names(browser: &Browser) -> Vec<&str> {
    browser
        .view
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect()
}

fn selected(browser: &Browser) -> &str {
    &browser.view.entries[browser.view.selected].name
}

#[test]
fn an_13_keys_and_escape_sequences_are_decoded() {
    assert_eq!(
        keys(b"jkgGlhsr?q\x03\r\x7f"),
        [
            Key::Down,
            Key::Up,
            Key::First,
            Key::Last,
            Key::Open,
            Key::Back,
            Key::Sort,
            Key::Refresh,
            Key::Help,
            Key::Quit,
            Key::Quit,
            Key::Open,
            Key::Back
        ]
    );
    assert_eq!(
        keys(b"\x1b[A\x1b[B\x1b[C\x1b[D\x1bOA\x1b[5~\x1b[6~\x1b[H\x1b[4~"),
        [
            Key::Up,
            Key::Down,
            Key::Open,
            Key::Back,
            Key::Up,
            Key::PageUp,
            Key::PageDown,
            Key::First,
            Key::Last
        ]
    );
    assert_eq!(keys(b"x\x1b\x1b[Zz"), [], "unknown bytes are ignored");
}

#[test]
fn an_12_entries_are_folders_and_files_largest_first() {
    let browser = browser();
    assert_eq!(names(&browser), ["big", "notes", "small"]);
    assert_eq!(selected(&browser), "big");
}

#[test]
fn an_13_moving_stays_within_the_list() {
    let mut browser = browser();
    browser.press(Key::Up);
    assert_eq!(selected(&browser), "big");
    browser.press(Key::Last);
    assert_eq!(selected(&browser), "small");
    browser.press(Key::Down);
    assert_eq!(selected(&browser), "small");
    browser.press(Key::First);
    browser.press(Key::PageDown);
    assert_eq!(selected(&browser), "small");
    browser.press(Key::PageUp);
    assert_eq!(selected(&browser), "big");
}

#[test]
fn an_13_open_loads_a_folder_and_back_returns_to_it() {
    let mut browser = browser();
    assert_eq!(
        browser.press(Key::Open),
        Effect::Load(PathBuf::from("/r/big"))
    );
    browser.show(vec![file("data", 50)]);
    assert_eq!(names(&browser), ["inner", "data"]);

    browser.press(Key::Back);
    assert_eq!(names(&browser), ["big", "notes", "small"]);
    assert_eq!(selected(&browser), "big", "back on the folder we came from");

    browser.press(Key::Back);
    assert_eq!(browser.path(), PathBuf::from("/r"), "never above the start");
}

#[test]
fn an_13_files_are_not_opened() {
    let mut browser = browser();
    browser.press(Key::Down);
    assert_eq!(selected(&browser), "notes");
    assert_eq!(browser.press(Key::Open), Effect::Redraw);
    assert_eq!(browser.path(), PathBuf::from("/r"));
}

#[test]
fn an_13_sort_switches_order_and_keeps_the_selection() {
    let mut browser = browser();
    browser.press(Key::Down);
    browser.press(Key::Sort);
    assert_eq!(names(&browser), ["big", "notes", "small"]);
    browser.press(Key::Last);
    browser.press(Key::Sort);
    assert_eq!(selected(&browser), "small");
    assert_eq!(browser.view.selected, 2);
}

#[test]
fn an_13_refresh_replaces_the_folder_and_fixes_the_totals_above() {
    let mut browser = browser();
    browser.press(Key::Open);
    browser.show(Vec::new());
    assert_eq!(
        browser.press(Key::Refresh),
        Effect::Measure(PathBuf::from("/r/big"))
    );

    browser.replace(folder_node("/r/big", 200, Vec::new()), Vec::new());

    assert!(names(&browser).is_empty());
    assert_eq!(browser.tree.bytes, 600);
    browser.press(Key::Back);
    assert_eq!(names(&browser), ["notes", "big", "small"]);
}

#[test]
fn an_13_help_shows_and_any_key_closes_it() {
    let mut browser = browser();
    browser.press(Key::Help);
    assert!(
        browser
            .lines(80, 20)
            .iter()
            .any(|line| line.contains("go up"))
    );
    assert_eq!(browser.press(Key::Down), Effect::Redraw);
    assert_eq!(
        selected(&browser),
        "big",
        "the closing key does nothing else"
    );
    assert_eq!(browser.press(Key::Quit), Effect::Quit);
}

#[test]
fn an_12_an_16_the_screen_fits_and_marks_the_selection() {
    let mut browser = browser();
    browser.set_rows(1);
    let lines = browser.lines(60, 3);
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("/r"), "{lines:?}");
    assert!(lines[1].starts_with("\x1b[7m"), "{lines:?}");
    assert!(lines[1].contains("big/"), "{lines:?}");
    assert!(lines[1].contains("60.0%"), "{lines:?}");
    assert!(lines[1].contains("[######    ]"), "{lines:?}");
    assert!(lines[2].starts_with("by size"), "{lines:?}");

    browser.press(Key::Down);
    let lines = browser.lines(60, 3);
    assert!(
        lines[1].contains("notes"),
        "scrolled to the selection: {lines:?}"
    );
    for width in [20, 40, 60] {
        for line in browser.lines(width, 3) {
            let visible = line.replace("\x1b[7m", "").replace("\x1b[0m", "");
            assert!(visible.chars().count() <= width, "{line:?}");
        }
    }
    let narrow = browser.lines(36, 3);
    assert!(!narrow[1].contains(']'), "no bar: {narrow:?}");
    assert!(narrow[1].contains("notes"), "the name stays: {narrow:?}");
}

#[test]
fn an_16_long_names_are_shortened_in_the_middle() {
    assert_eq!(shorten("abcdefghij", 7), "abc…hij");
    assert_eq!(shorten("abc", 7), "abc");
    assert_eq!(shorten("abcdef", 1), "…");
    assert_eq!(shorten("abcdef", 0), "");
}

/// A browser over a real temp folder, so marks can record real candidates.
fn on_disk() -> (tempfile::TempDir, Browser) {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().to_owned();
    std::fs::create_dir(root.join("big")).unwrap();
    std::fs::write(root.join("notes"), b"x").unwrap();
    let tree = folder_node(
        &root.to_string_lossy(),
        1000,
        vec![folder_node("big", 600, Vec::new())],
    );
    let browser = Browser::new(root, tree, vec![file("notes", 300)]);
    (dir, browser)
}

fn mark_selected(browser: &mut Browser) {
    match browser.press(Key::Mark) {
        Effect::Mark(path) => browser.mark(Candidate::scan(path).unwrap()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_18_space_and_d_are_mark_and_trash() {
    assert_eq!(keys(b" d"), [Key::Mark, Key::Trash]);
}

#[test]
fn an_18_an_19_marks_toggle_show_and_stay_across_folders() {
    let (_dir, mut browser) = on_disk();
    mark_selected(&mut browser);
    assert_eq!(selected(&browser), "notes", "marking moves down");
    let lines = browser.lines(80, 6);
    assert!(lines[1].contains("*"), "{lines:?}");
    assert!(lines[5].starts_with("1 marked, 600 B"), "{lines:?}");

    browser.press(Key::Up);
    assert_eq!(
        browser.press(Key::Mark),
        Effect::Redraw,
        "unmarking needs no scan"
    );
    assert!(
        browser.picked().len() == 1,
        "nothing marked: the selection is picked"
    );
    assert!(!browser.lines(80, 6)[5].contains("marked"));

    // Unmarking moved down too, so this marks `notes`; then visit big/ and come back.
    assert_eq!(selected(&browser), "notes");
    mark_selected(&mut browser);
    browser.press(Key::First);
    browser.press(Key::Open);
    browser.show(Vec::new());
    browser.press(Key::Back);
    assert_eq!(browser.picked().len(), 1);
    assert!(browser.picked()[0].path().ends_with("notes"));
}

#[test]
fn an_20_d_picks_the_marks_or_else_the_selection() {
    let (dir, mut browser) = on_disk();
    assert_eq!(browser.press(Key::Trash), Effect::Trash);
    let picked = browser.picked();
    assert_eq!(picked.len(), 1);
    assert_eq!(picked[0].path(), dir.path().join("big"));

    mark_selected(&mut browser);
    mark_selected(&mut browser);
    let paths: Vec<_> = browser
        .picked()
        .iter()
        .map(|c| c.path().to_owned())
        .collect();
    assert_eq!(paths, [dir.path().join("big"), dir.path().join("notes")]);
    browser.clear_marks();
    assert_eq!(browser.picked().len(), 1);
}

#[test]
fn an_22_the_folder_to_measure_again_holds_every_trashed_path() {
    let browser = browser();
    let root = PathBuf::from("/r");
    assert_eq!(
        browser.common_trail(&[root.join("big/inner/a"), root.join("big/inner/b")]),
        ["big", "inner"]
    );
    assert_eq!(
        browser.common_trail(&[root.join("big/inner/a"), root.join("small/x")]),
        Vec::<String>::new()
    );
    assert_eq!(
        browser.common_trail(&[root.join("notes")]),
        Vec::<String>::new()
    );
}

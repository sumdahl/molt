//! Tests for the completion generators (CO-02 to CO-04), on a small command tree.

use clap::{Arg, ArgAction, Command};

use super::*;

fn command() -> Command {
    Command::new("onyx")
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .global(true)
                .action(ArgAction::SetTrue),
        )
        .subcommand(
            Command::new("system").arg(
                Arg::new("only")
                    .long("only")
                    .num_args(1..)
                    .value_parser(["pacman-cache", "journal"]),
            ),
        )
        .subcommand(Command::new("analyze").arg(Arg::new("path")))
        .subcommand(Command::new("secret").hide(true))
        .subcommand(
            Command::new("completions")
                .arg(Arg::new("shell").value_parser(["bash", "zsh", "fish"])),
        )
}

#[test]
fn co_02_the_tree_has_every_visible_subcommand_its_flags_and_globals() {
    let tree = Tree::from(command());
    let names: Vec<&str> = tree
        .subcommands
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(names, ["system", "analyze", "completions", "help"]);
    let system = &tree.subcommands[0].1;
    let longs: Vec<&str> = system.flags.iter().map(|flag| flag.long.as_str()).collect();
    assert!(
        longs.contains(&"only") && longs.contains(&"dry-run") && longs.contains(&"help"),
        "{longs:?}"
    );
    let only = system
        .flags
        .iter()
        .find(|flag| flag.long == "only")
        .unwrap();
    assert_eq!(
        only.values,
        Some(vec!["pacman-cache".to_owned(), "journal".to_owned()])
    );
    let dry = system
        .flags
        .iter()
        .find(|flag| flag.long == "dry-run")
        .unwrap();
    assert_eq!(dry.values, None, "a switch takes no value");
    assert!(
        tree.subcommands[1].1.files,
        "analyze's path completes files"
    );
    assert_eq!(tree.subcommands[2].1.positional, ["bash", "zsh", "fish"]);
}

#[test]
fn co_02_co_03_every_script_names_every_word() {
    for shell in Shell::ALL {
        let script = script(shell, command());
        for word in [
            "system",
            "analyze",
            "completions",
            "--only",
            "--dry-run",
            "pacman-cache",
            "journal",
            "zsh",
        ] {
            let word = if shell == Shell::Fish {
                word.trim_start_matches("--")
            } else {
                word
            };
            assert!(script.contains(word), "{shell:?} lacks {word}:\n{script}");
        }
        assert!(
            !script.contains("secret"),
            "{shell:?} shows a hidden command"
        );
    }
}

#[test]
fn co_04_every_word_is_plain_so_nothing_needs_quoting() {
    assert!(plain("pacman-cache") && plain("python-venv") && plain("dry-run"));
    for bad in ["a b", "$(x)", "a'b", "`x`", ""] {
        assert!(!plain(bad), "{bad:?}");
    }
    // A tree with an unsafe word leaves that word out.
    let tree = Tree::from(Command::new("onyx").subcommand(
        Command::new("ok").arg(Arg::new("v").long("v").value_parser(["fine", "$(rm)"])),
    ));
    assert_eq!(
        tree.subcommands[0].1.flags[0].values,
        Some(vec!["fine".to_owned()])
    );
}

//! `onyx completions` (specs/018-completions.md): static completion scripts for bash,
//! zsh and fish, generated from clap's own command tree, with no completion crate.

use std::fmt::Write;

use clap::{Command, ValueEnum};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    pub const ALL: [Self; 3] = [Self::Bash, Self::Zsh, Self::Fish];
}

/// A long flag, and what it takes: `None` for a switch, else its fixed values (empty
/// when any value goes).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Flag {
    long: String,
    values: Option<Vec<String>>,
}

/// What can follow a command (CO-02).
#[derive(Debug, Clone, Default)]
struct Node {
    flags: Vec<Flag>,
    /// A positional argument's fixed values.
    positional: Vec<String>,
    /// A positional argument with no fixed values: the shell's file completion.
    files: bool,
}

#[derive(Debug)]
struct Tree {
    root: Node,
    subcommands: Vec<(String, Node)>,
}

/// CO-04: words the scripts use unquoted. Anything else is left out.
fn plain(word: &str) -> bool {
    !word.is_empty()
        && word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

impl Node {
    fn from(command: &Command) -> Self {
        let mut node = Self::default();
        for arg in command.get_arguments().filter(|arg| !arg.is_hide_set()) {
            let values: Vec<String> = arg
                .get_possible_values()
                .iter()
                .filter(|value| !value.is_hide_set())
                .map(|value| value.get_name().to_owned())
                .filter(|value| plain(value))
                .collect();
            match arg.get_long() {
                Some(long) if plain(long) => node.flags.push(Flag {
                    long: long.to_owned(),
                    // A switch's parser has true/false values, so ask whether it takes any.
                    values: arg.get_action().takes_values().then_some(values),
                }),
                Some(_) => {}
                None if arg.is_positional() && values.is_empty() => node.files = true,
                None if arg.is_positional() => node.positional = values,
                None => {}
            }
        }
        node
    }
}

impl From<Command> for Tree {
    fn from(mut command: Command) -> Self {
        // Propagates the global flags and adds `help` and `--help`.
        command.build();
        Self {
            root: Node::from(&command),
            subcommands: command
                .get_subcommands()
                .filter(|sub| !sub.is_hide_set() && plain(sub.get_name()))
                .map(|sub| (sub.get_name().to_owned(), Node::from(sub)))
                .collect(),
        }
    }
}

impl Tree {
    fn names(&self) -> String {
        let names: Vec<&str> = self
            .subcommands
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        names.join(" ")
    }
}

impl Node {
    fn longs(&self) -> String {
        let longs: Vec<String> = self
            .flags
            .iter()
            .map(|flag| format!("--{}", flag.long))
            .collect();
        longs.join(" ")
    }

    /// Flags whose values come from a fixed list, and flags that take a free value.
    fn valued(&self) -> (Vec<&Flag>, Vec<&Flag>) {
        self.flags
            .iter()
            .filter(|flag| flag.values.is_some())
            .partition(|flag| {
                flag.values
                    .as_ref()
                    .is_some_and(|values| !values.is_empty())
            })
    }
}

/// CO-01: the script for `shell`.
pub fn script(shell: Shell, command: Command) -> String {
    let tree = Tree::from(command);
    match shell {
        Shell::Bash => bash(&tree),
        Shell::Zsh => zsh(&tree),
        Shell::Fish => fish(&tree),
    }
}

/// The `case "$flag"` block bash and zsh share: after a flag with fixed values, `offer`
/// completes them unless a new flag is being typed; after one with a free value, nothing.
fn flag_cases(out: &mut String, node: &Node, offer: impl Fn(&str) -> String) {
    let (listed, free) = node.valued();
    if listed.is_empty() && free.is_empty() {
        return;
    }
    let _ = writeln!(out, "            case \"$flag\" in");
    for flag in listed {
        let values = flag.values.as_deref().unwrap_or_default().join(" ");
        let _ = writeln!(
            out,
            "                --{}) [[ \"$cur\" == -* ]] || {{ {}; return; }} ;;",
            flag.long,
            offer(&values)
        );
    }
    for flag in free {
        let _ = writeln!(
            out,
            "                --{}) [[ \"$flag\" == \"$prev\" ]] && return ;;",
            flag.long
        );
    }
    let _ = writeln!(out, "            esac");
}

/// One `case` branch per command: `$flag` is the last flag typed, `$cur` the word.
fn bash_branch(out: &mut String, pattern: &str, node: &Node, extra: &str) {
    let _ = writeln!(out, "        {pattern})");
    flag_cases(out, node, |values| {
        format!("COMPREPLY=($(compgen -W \"{values}\" -- \"$cur\"))")
    });
    let words = if node.positional.is_empty() {
        format!("{extra} {}", node.longs())
    } else {
        node.positional.join(" ")
    };
    if node.files {
        // Empty, so `-o default` falls back to the shell's file completion.
        let _ = writeln!(
            out,
            "            [[ \"$cur\" == -* ]] && COMPREPLY=($(compgen -W \"{}\" -- \"$cur\")) ;;",
            node.longs()
        );
    } else {
        let _ = writeln!(
            out,
            "            COMPREPLY=($(compgen -W \"{}\" -- \"$cur\")) ;;",
            words.trim()
        );
    }
}

fn bash(tree: &Tree) -> String {
    let mut out = String::from(
        "# bash completion for onyx, made by `onyx completions bash`.\n\
         _onyx() {\n\
         \x20   local cur=\"${COMP_WORDS[COMP_CWORD]}\" prev=\"${COMP_WORDS[COMP_CWORD-1]}\"\n\
         \x20   local cmd=\"\" flag=\"\" i j\n\
         \x20   for ((i = 1; i < COMP_CWORD; i++)); do\n\
         \x20       case \"${COMP_WORDS[i]}\" in -*) ;; *) cmd=\"${COMP_WORDS[i]}\"; break ;; esac\n\
         \x20   done\n\
         \x20   for ((j = COMP_CWORD - 1; j > i; j--)); do\n\
         \x20       case \"${COMP_WORDS[j]}\" in -*) flag=\"${COMP_WORDS[j]}\"; break ;; esac\n\
         \x20   done\n\
         \x20   COMPREPLY=()\n\
         \x20   case \"$cmd\" in\n",
    );
    bash_branch(&mut out, "\"\"", &tree.root, &tree.names());
    for (name, node) in &tree.subcommands {
        bash_branch(&mut out, name, node, "");
    }
    out.push_str("    esac\n}\ncomplete -o default -F _onyx onyx\n");
    out
}

fn zsh(tree: &Tree) -> String {
    let mut out = String::from(
        "#compdef onyx\n\
         # zsh completion for onyx, made by `onyx completions zsh`.\n\
         _onyx() {\n\
         \x20   local cur=\"${words[CURRENT]}\" prev=\"${words[CURRENT-1]}\"\n\
         \x20   local cmd=\"\" flag=\"\" i j\n\
         \x20   for ((i = 2; i < CURRENT; i++)); do\n\
         \x20       case \"${words[i]}\" in -*) ;; *) cmd=\"${words[i]}\"; break ;; esac\n\
         \x20   done\n\
         \x20   for ((j = CURRENT - 1; j > i; j--)); do\n\
         \x20       case \"${words[j]}\" in -*) flag=\"${words[j]}\"; break ;; esac\n\
         \x20   done\n\
         \x20   case \"$cmd\" in\n",
    );
    let branch = |out: &mut String, pattern: &str, node: &Node, extra: &str| {
        let _ = writeln!(out, "        {pattern})");
        flag_cases(out, node, |values| format!("compadd -- {values}"));
        if node.files {
            let _ = writeln!(
                out,
                "            if [[ \"$cur\" == -* ]]; then compadd -- {}; else _files; fi ;;",
                node.longs()
            );
        } else if node.positional.is_empty() {
            let _ = writeln!(out, "            compadd -- {extra} {} ;;", node.longs());
        } else {
            let _ = writeln!(
                out,
                "            compadd -- {} ;;",
                node.positional.join(" ")
            );
        }
    };
    branch(&mut out, "\"\"", &tree.root, &tree.names());
    for (name, node) in &tree.subcommands {
        branch(&mut out, name, node, "");
    }
    out.push_str("    esac\n}\n_onyx \"$@\"\n");
    out
}

fn fish(tree: &Tree) -> String {
    let mut out = String::from(
        "# fish completion for onyx, made by `onyx completions fish`.\ncomplete -c onyx -f\n",
    );
    let _ = writeln!(
        out,
        "complete -c onyx -n __fish_use_subcommand -a '{}'",
        tree.names()
    );
    for flag in &tree.root.flags {
        let _ = writeln!(
            out,
            "complete -c onyx -n __fish_use_subcommand -l {}",
            flag.long
        );
    }
    for (name, node) in &tree.subcommands {
        let when = format!("__fish_seen_subcommand_from {name}");
        for flag in &node.flags {
            let _ = match &flag.values {
                Some(values) if !values.is_empty() => writeln!(
                    out,
                    "complete -c onyx -n '{when}' -l {} -xa '{}'",
                    flag.long,
                    values.join(" ")
                ),
                Some(_) => writeln!(out, "complete -c onyx -n '{when}' -l {} -x", flag.long),
                None => writeln!(out, "complete -c onyx -n '{when}' -l {}", flag.long),
            };
        }
        if node.files {
            let _ = writeln!(out, "complete -c onyx -n '{when}' -F");
        } else if !node.positional.is_empty() {
            let _ = writeln!(
                out,
                "complete -c onyx -n '{when}' -a '{}'",
                node.positional.join(" ")
            );
        }
    }
    out
}

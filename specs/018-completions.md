# 018: Shell completions

Status: Draft (#59)
Prefix: CO

## Goal

`molt completions <shell>` prints a completion script for bash, zsh or fish, so Tab completes molt's commands, flags and fixed ids. The script is generated from clap's own command tree, so it can't drift from the real flags, with no completion crate (no new dependencies).

## Non-goals

- Installing the script. molt prints it; the user or a package (#62) puts it where the shell looks.
- Dynamic completion. The script never runs molt when you press Tab, so a keypress can't execute anything.
- Completing paths for `analyze` beyond the shell's own file completion, or app names for `uninstall`.

## Requirements

- **CO-01** `molt completions <bash|zsh|fish>` prints the script on stdout and exits 0. Another shell name is a usage error (exit 2). It reads no config, writes nothing, and runs even when the config is broken.
- **CO-02** The script is built from clap's command tree at run time: every subcommand, its long flags (the global ones too), and, for flags that take a value from a fixed list, that list. `molt completions` itself is included. Hidden items aren't.
- **CO-03** The fixed lists are molt's own: `system`'s and `optimize`'s task ids for `--only` and `--skip`, `purge`'s project kinds, and the shell names. They come from the same constants the commands check against, so they can't drift.
- **CO-04** The script is static. It defines completion functions and registers them, and runs no command except the shell's own builtins (and `compgen` and file completion in bash).
- **CO-05** README says where each shell looks: `~/.local/share/bash-completion/completions/molt`, `~/.zfunc/_molt` (with `fpath`), and `~/.config/fish/completions/molt.fish`.

## Test strategy

- Unit tests: every subcommand and every long flag of the command tree is in each script; the fixed lists are there.
- Smoke tests: `bash -n` parses the bash script; sourcing it in a temp-home bash and calling its function for `molt cl`, `molt system --only `, and `molt purge --skip c` gives `clean`, the task ids, and `cargo`. `zsh -n` and `fish --no-execute` check theirs where installed (skipped otherwise; CI installs both).
- Unknown shell exits 2; a broken config still completes.

## Decisions

- **Hand-written generators** for three shells, about a hundred lines, instead of `clap_complete` (no new dependencies).
- **Fixed lists through clap's possible values.** The `--only`/`--skip` flags get their ids as possible values, which also makes clap reject a wrong id with its own usage error (exit 2), as SY-01 and OP-01 already require.

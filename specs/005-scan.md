# 005: `molt scan`

Status: Draft
Prefix: SC

## Goal

`molt scan` tells the user how much space molt could free, and where, without changing anything. Its numbers are exactly what `clean` would act on, because it asks safe delete for a dry run of the same plan instead of measuring separately.

## Non-goals

- Choosing or removing anything. That is `clean` (006).
- Project build folders (`purge`, 008) and installers (`installer`, 009). They get their own commands.
- Progress bars and an interactive view.

## Model

1. Start up as every command does (004).
2. Match the enabled rules (002) to get candidates, skips and inactive rules.
3. Run the candidates through safe delete as a dry run, one plan per mode (permanent, trash). Safe delete measures sizes, applies the whitelist and the protected paths, and counts hard links once (SD-12, SD-13).
4. Group the results by category, then by rule, and print them.

## Requirements

- **SC-01** `scan` changes nothing. It never runs a live plan, writes no operation-log entry, creates no trash directory, and runs no external command. `--live` has no effect on it.
- **SC-02** Each rule's size is the sum of its candidates' `would-remove` / `would-trash` bytes from the safe-delete dry run. Candidates that safe delete would skip (protected, whitelisted, mount point…) are not counted. They are listed with their reason.
- **SC-03** The report groups rules by category. Categories, and rules within a category, are sorted by size, largest first. Rules with no matches are left out. Inactive rules (RE-08) are counted in one summary line.
- **SC-04** Sizes are shown with binary units, the way `du -h` and `df -h` show them (`512 B`, `1.5 KiB`, `23.4 GiB`), with one decimal from KiB up.
- **SC-05** The report ends with a total and says which command would free the space (`molt clean`). A rule with `clean_with` (P-07) shows that command next to its size, since `clean` will run it instead of deleting files.
- **SC-06** With `--json`, the report is one JSON object on stdout: the total, and for each rule its id, name, category, mode, bytes, `clean_with`, and items (path, bytes, outcome or skip reason), plus the rule-engine skips (RE-09, RE-10) and the inactive rule ids. The shape is a stable interface (P-11).
- **SC-07** Matches the rules engine skipped (symlinks, unreadable directories) are reported, not counted. If any directory couldn't be read, the exit code is 3 (P-13) and the report says the scan is partial (P-12).
- **SC-08** The global flags work after the command name: `molt scan --json` is the same as `molt --json scan` (CL-01).

## Test strategy

- Smoke tests run the real binary in a temp home (as 004) with files laid out to match built-in and user rules, and assert on the `--json` output.
- SC-01: snapshot the whole temp home, including the state and trash directories, before and after `molt scan`, `molt scan --live` and `molt scan --dry-run`.
- SC-04 is a table test of the size formatter.

## Open questions

- **What `molt` with no command does.** Run `scan`, or keep printing help? (Proposal: run `scan`. It's read-only, so it's the safe default, and it's the first thing anyone wants. The interactive menu (phase 5) can take over later.)
- **Units.** Binary units (`GiB`, matching `du -h`/`df -h`) or decimal (`GB`, matching disk labels and most file managers)? (Proposal: binary, as SC-04.)
- **Detail.** Should the human-readable report list every path, or only one line per rule? (Proposal: one line per rule, with paths in `--json`. `node_modules`-style rules can have thousands of paths.)

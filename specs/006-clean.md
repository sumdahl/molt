# 006: `molt clean`

Status: Accepted
Prefix: CN

## Goal

`molt clean` frees the space `molt scan` found. It always shows the full dry run first (P-14), asks before changing anything (P-06), runs a tool's own cleanup command when it has one (P-07), leaves alone the data of programs that are running (P-08), and reports exactly what it freed.

## Non-goals

- System data (pacman cache, journal). That is the system step (007, #7).
- A full-screen picker. The interactive menu is phase 5 (#15); here, the user picks with flags and a yes/no question.
- Emptying the trash (010 Decisions).

## Model

1. Start up (004) and match the enabled rules (002), narrowed by `--only` and `--skip` (CN-02).
2. Leave out rules whose programs are running (CN-03).
3. **Preview** (P-14): a safe-delete dry run of every remaining candidate, shown like `scan`, plus the exact command line for each rule that has `clean_with`, and the locked folders (LF-06).
4. **Ask** (P-06): one yes/no for the whole preview, default no. Then, if there are locked folders, one more yes/no to open them (LF-05), default no.
5. **Act**: for each rule, run its `clean_with` command, or else run its candidates through safe delete, live, with the modes the preview showed.
6. **Report** what was freed, per rule, and the total.

## Requirements

### Choosing
- **CN-01** With no flags, `clean` acts on every enabled, applicable rule with matches, as `scan` shows them.
- **CN-02** `--only <id>…` limits the run to the named rules; `--skip <id>…` leaves the named rules out. An id that names no loaded rule is a usage error (exit 2), as a typo would otherwise clean more than meant.
- **CN-03** A rule whose `processes` (P-08) include a running program is left out, and the preview names the rule and the program. Running programs are read from `/proc/*/comm` and `/proc/*/cmdline` (the executable's base name).

### Preview and consent
- **CN-04** The live run prints the full preview before the first change: every rule, its size, its mode (rules going to the trash say so), and the command it would run (P-14). With `--json`, the preview is a JSON object on stdout before the result.
- **CN-05** Interactive runs ask once, default no. Anything but `y` or `yes` changes nothing, and molt exits 0 with "Nothing changed." A run without a terminal on stdin needs `--yes`; without it, molt prints the preview and exits 1 with nothing changed.
- **CN-06** The locked-folder consent (LF-05) is a second question, after the first yes, default no. Non-interactive runs need `--fix-permissions`. Without consent, locked folders are kept (LF-01).
- **CN-07** `--dry-run` (or a dry run from the config or `MOLT_DRY_RUN`, with the CL-06 notice) prints the preview and stops, changing nothing.

### Acting
- **CN-08** A rule with `clean_with` runs that command, with no shell, `$PATH` limited to its absolute entries (SC-10), stdin closed, and its output shown. molt never also deletes that rule's files. If the command fails (non-zero exit, or it can't start), the rule is reported as failed and its files are left alone (P-02). Its freed size is measured as the rule's size before minus after.
- **CN-09** Owning-tool commands run first. Every other rule's candidates go through one live safe-delete plan per mode, with the identity recorded by the preview's scan, so anything that changed since the preview is skipped (SD-08, SD-10).
- **CN-10** `--permanent` and `--trash` override the mode of every rule in the run (000 Decisions). They conflict with each other (exit 2).

### Reporting
- **CN-11** The result lists, per rule, what was freed and what was skipped or failed and why, and the total freed. The exit code is 0 when everything planned was done, 3 when anything failed or was kept (P-13).
- **CN-12** Every change is in the operation log (SD-15). An owning-tool command is logged too: its command line, exit status, and the bytes freed.

## Test strategy

- Smoke tests run the real binary in a temp home with `--yes` (and stdin from a pipe for the interactive cases), as 004 and 005 do. Each checks the files that were removed and those that must remain.
- CN-04: the preview text appears on stdout before any change. A rule with a fake `clean_with` tool records when it ran; the test checks the preview was already printed.
- CN-03: a test starts a dummy process named like a rule's `processes` (a copy of `sleep` with that name) and checks the rule is left out.
- CN-08: fake tools on a temp `$PATH`, one that succeeds and one that fails.
- P-05: `clean --dry-run` and `clean` answered "no" leave a snapshot of the temp home unchanged.

## Decisions

- **Flags plus one question** (CN-02, CN-05) until the phase 5 menu.
- **A failing owning tool is reported, never worked around** (CN-08, P-02).
- **No terminal and no `--yes`: preview, change nothing, exit 1** (CN-05).

# 022: Fingerprint confirmation

Status: Implemented (#65)
Prefix: FP

## Goal

On a machine with a fingerprint reader set up through fprintd, a live run can be confirmed by touching the sensor instead of typing `y`. It's a convenience for the same person at the keyboard, not extra security: it's off unless the config turns it on, never required, and any doubt falls back to the typed question.

## Non-goals

- Using the fingerprint as a security barrier (e.g. against someone else at an unlocked session). Typing `y` still works whenever the fingerprint doesn't.
- Enrolling fingers, or talking to fprintd's D-Bus API directly. `fprintd-verify` is the owning tool (P-07); no new dependencies.
- Replacing `sudo`'s own authentication for `system`, `optimize` and `uninstall`.
- The `analyze` browser's own in-screen question for moving marked items to the trash (AN-21), which is answered inside its full-screen view.

## Requirements

- **FP-01** A config key `confirm = "type"` (the default) or `confirm = "fingerprint"`. Any other value is a config error (CF-03). A missing key means `"type"`.
- **FP-02** The fingerprint is offered only where the typed question would be asked today: a live run, on a terminal, without `--yes`. The full dry run is shown first (P-14), exactly as now. `--yes`, dry runs, `--json` and runs without a terminal behave as they do today.
- **FP-03** With `confirm = "fingerprint"`, onyx prints the question and `Touch the fingerprint sensor to go ahead, or press Ctrl-C to stop.`, then runs `fprintd-verify` (found on the absolute `$PATH` entries, SC-10) for the current user, with no shell, its stdin closed, and its output hidden. Exit status 0 means yes.
- **FP-04** Anything else falls back to the typed question, default no, with one line saying why: `fprintd-verify` isn't installed, it exits non-zero (no reader, no enrolled finger, no match, its own timeout), or it can't be started. A failed fingerprint never counts as yes, and never as a final no either: the typed question decides.
- **FP-05** Ctrl-C while waiting for the finger stops onyx with nothing changed, as Ctrl-C at the typed question does.
- **FP-06** Each yes, before anything changes, adds a confirmation entry to the operation log: `{"time", "confirmed": "typed" | "fingerprint" | "yes-flag", "question"}`. Most live runs (`clean`, `purge`…) write only item entries, so the confirmation gets its own entry rather than a field on a command entry. Like any log write, a failure stops the run with nothing changed (SD-15). `ONYX_NO_OPLOG=1` skips it. `onyx history` skips these entries without counting them as unreadable (HI-05).

## Test strategy

- No test touches a real reader or fprintd. A fake `fprintd-verify` on a temp `$PATH` exits 0, exits 1, or sleeps; pty smoke tests in a temp home check:
  - FP-03: exit 0 goes ahead without a typed `y`; the log says `fingerprint`.
  - FP-04: exit 1, or no `fprintd-verify`, falls back to the typed question, and `n` (or Enter) changes nothing.
  - FP-02: `--yes`, `--dry-run` and piped stdin never run `fprintd-verify` (the fake records each call).
  - FP-01: a bad `confirm` value is a config error that stops onyx (exit 1).
- What only real hardware can show (that `fprintd-verify` behaves this way with a real sensor) is stated as unverified in the PR; this machine has no reader.

## Decisions

- **A failed fingerprint falls back to the typed question** (maintainer, 2026-10-31): it's a convenience, not security, so a dirty sensor or a missing reader never blocks a run, and never counts as yes.
- **Each confirmation is logged** (maintainer, 2026-10-31), with how it was given.
- **Ctrl-C needs no handler** (FP-05): the terminal sends SIGINT to onyx and `fprintd-verify` together, and onyx stops before changing anything, as it does at the typed question.

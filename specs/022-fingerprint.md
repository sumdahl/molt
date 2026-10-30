# 022: Fingerprint confirmation

Status: Draft (#65)
Prefix: FP

## Goal

On a machine with a fingerprint reader set up through fprintd, a live run can be confirmed by touching the sensor instead of typing `y`. It's a convenience for the same person at the keyboard, not extra security: it's off unless the config turns it on, never required, and any doubt falls back to the typed question.

## Non-goals

- Using the fingerprint as a security barrier (e.g. against someone else at an unlocked session). Typing `y` still works whenever the fingerprint doesn't.
- Enrolling fingers, or talking to fprintd's D-Bus API directly. `fprintd-verify` is the owning tool (P-07); no new dependencies.
- Replacing `sudo`'s own authentication for `system`, `optimize` and `uninstall`.

## Requirements

- **FP-01** A config key `confirm = "type"` (the default) or `confirm = "fingerprint"`. Any other value is a config error (CF-03). A missing key means `"type"`.
- **FP-02** The fingerprint is offered only where the typed question would be asked today: a live run, on a terminal, without `--yes`. The full dry run is shown first (P-14), exactly as now. `--yes`, dry runs, `--json` and runs without a terminal behave as they do today.
- **FP-03** With `confirm = "fingerprint"`, onyx prints the question and `Touch the fingerprint sensor to go ahead, or press Ctrl-C to stop.`, then runs `fprintd-verify` (found on the absolute `$PATH` entries, SC-10) for the current user, with no shell, its stdin closed, and its output hidden. Exit status 0 means yes.
- **FP-04** Anything else falls back to the typed question, default no, with one line saying why: `fprintd-verify` isn't installed, it exits non-zero (no reader, no enrolled finger, no match, its own timeout), or it can't be started. A failed fingerprint never counts as yes, and never as a final no either: the typed question decides.
- **FP-05** Ctrl-C while waiting for the finger stops onyx with nothing changed, as Ctrl-C at the typed question does.
- **FP-06** The operation log records how the run was confirmed (`"confirmed": "typed" | "fingerprint" | "yes-flag"`) on the command entry, so `onyx history` can show it later.

## Test strategy

- No test touches a real reader or fprintd. A fake `fprintd-verify` on a temp `$PATH` exits 0, exits 1, or sleeps; pty smoke tests in a temp home check:
  - FP-03: exit 0 goes ahead without a typed `y`; the log says `fingerprint`.
  - FP-04: exit 1, or no `fprintd-verify`, falls back to the typed question, and `n` (or Enter) changes nothing.
  - FP-02: `--yes`, `--dry-run` and piped stdin never run `fprintd-verify` (the fake records each call).
  - FP-01: a bad `confirm` value is a config error that stops onyx (exit 1).
- What only real hardware can show (that `fprintd-verify` behaves this way with a real sensor) is stated as unverified in the PR; this machine has no reader.

## Open questions

1. **A failed fingerprint falls back to typing `y` (FP-04).** Recommended, since this is convenience and not security. The alternative, stopping at a failed match, would make a dirty sensor block a run.
2. **The log field (FP-06).** Recommended, so it's clear later which runs were confirmed by finger. Drop it if you'd rather keep the log format as it is.

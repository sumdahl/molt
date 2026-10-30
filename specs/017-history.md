# 017: `molt history`

Status: Implemented
Prefix: HI

## Goal

`molt history` shows what molt has done to this machine: every item it moved to the trash or deleted, and every command it ran through `sudo` or as you, newest first, with sizes. It reads the operation log (P-09, SD-15) and changes nothing.

## Non-goals

- Undoing anything. Trashed items come back through the trash, as any other trashed file does; history says which entries those are.
- Editing, rotating or clearing the log. It's the record of what molt removed.
- History of other tools.

## Model

1. Read `$XDG_STATE_HOME/molt/operations.log` (the path the log is written to, whatever `MOLT_NO_OPLOG` says), one JSON object per line.
2. Keep the entries `--since` allows, newest first, at most `--limit`.
3. Print them, then a total. Nothing is written, and no folder is created.

## Requirements

- **HI-01** `molt history` only reads. It never writes, creates or locks a file, so `--dry-run` changes nothing about it, and it needs no question.
- **HI-02** Each item entry is shown as its local time to the minute, what happened (`trashed`, `deleted`, or `partly deleted` for a failure that removed some of it), its size (P-12's units), and its path. Each command entry is shown as its time, `ran`, its size when it freed some, its full command line, and its exit status when that wasn't 0 ("didn't run" when it has none).
- **HI-03** Entries are newest first. `--limit <N>` (default 50, at least 1) caps how many are shown, and `--all` shows every one; the footer says how many were left out.
- **HI-04** `--since <date>` (`YYYY-MM-DD`, local midnight) keeps entries from that day on. A bad date is a usage error (exit 2).
- **HI-05** The footer gives the total size of the shown entries that freed space, and how many of them went to the trash, so they can be restored from there.
- **HI-06** A line that isn't a valid entry is skipped and counted, and the footer says so; the run exits 3 (P-13: partial). A missing log means "Nothing in the history yet" and exit 0. A log that can't be read is an error (exit 1).
- **HI-07** `--json` prints `{"entries": [...], "skipped_lines": N, "left_out": N}`, each entry as it is in the log, newest first, after `--since` and `--limit`.

## Test strategy

- Smoke tests on a temp home whose log is written by a real `molt clean --yes` and `molt installer --yes`, plus fake command entries and a corrupt line appended by the test.
- HI-01: a snapshot of the temp home is unchanged by `history`, `history --dry-run`, and `history` with no log (the state folder isn't created).
- HI-02, HI-03: the order and the lines; `--limit 1` shows one and says how many were left out.
- HI-04: `--since` in the future shows none; `--since 2026-13-01` exits 2.
- HI-06: the corrupt line is counted and the exit is 3.
- HI-07: the JSON shape.

## Decisions

- **Local time, to the minute.** The log keeps UTC to the nanosecond; people read local time.
- **Default 50.** Enough to see the last few runs without scrolling a long log away.
- **No trash lookup.** Whether a trashed item is still in the trash would need reading every trash's `info` files; the footer points at the trash instead.

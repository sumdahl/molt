# 012: `molt analyze`

Status: Draft
Prefix: AN

## Goal

`molt analyze` shows where the space in a folder goes, largest first, so you can find what `scan` doesn't know about. This spec covers the measuring core and its plain output (#33). The full-screen browser (#34) and trashing picked items (#35) extend it later, with requirements added here.

## Non-goals

- Changing anything. `analyze` is read-only; trashing comes with #35, through safe delete.
- A full-screen interface. That's #34.
- Apparent sizes (`du --apparent-size`). molt reports on-disk sizes only (P-12).

## Model

1. Start up (004).
2. Walk the folder given (default: the current directory), in parallel, without following symlinks or crossing filesystems.
3. Print its children by on-disk size, largest first, down to `--depth` levels.

## Requirements

### Measuring
- **AN-01** Sizes are on-disk (`st_blocks × 512`), and a file with several hard links inside the walk is counted once, for the first path that reaches it (P-12).
- **AN-02** The walk never follows symlinks (a symlink counts as its own size) and never crosses into another filesystem: a mount point inside is listed with size 0 and marked `mount`.
- **AN-03** A folder that can't be read, or anything that can't be measured, makes the folders above it partial: their sizes are shown with `+` (at least this much), and `--json` sets `partial` on them. molt exits 3 if anything was partial (P-13).
- **AN-04** The walk runs in parallel on up to the machine's available parallelism, with the standard library's threads (no new dependency). The result doesn't depend on the order threads finish in.
- **AN-05** `PATH` must be a directory. A missing path, a file, or a symlink is an error (exit 1) naming the path.

### Output
- **AN-06** The text output lists the folder's children, largest first, then by name: size, the share of the total, and the name, with `/` after folders. `--depth N` (default 1) also lists the children of folders, indented, down to N levels.
- **AN-07** Each level shows its 20 largest entries, then one line for the rest ("… 37 more, 1.2 GiB"). `--all` shows every entry.
- **AN-08** `--json` prints one object: the path, size, `partial`, and `children` down to `--depth`, each with name, kind (`dir`, `file`, `symlink`, `mount`, `other`), size and `partial`. `--all` doesn't apply; JSON has everything to that depth.
- **AN-09** `--dry-run` and the config's dry run are accepted and change nothing, as `analyze` changes nothing anyway (P-05). No dry-run notice is printed.

## Test strategy

- Unit tests on a temp folder with known files: sizes match `st_blocks`, a hard link pair counts once, a symlink counts as itself, an unreadable folder (`chmod 000`) makes its parents partial, and the result is the same over many runs (AN-04).
- AN-02: a tmpfs mounted inside the temp folder in a user namespace, as 001's tests do; skipped where that isn't allowed.
- Smoke tests: text and `--json` shape, `--depth`, the 20-entry cut and `--all`, exit 3 on partial, exit 1 on a bad path.
- P-05: a snapshot of the temp folder is unchanged after a run.

## Open questions

1. **Default folder.** The current directory (proposed, like `du` and `ncdu`), or `~`?
2. **Depth.** Default 1 (proposed: the folder's children), or 2?
3. **The 20-entry cut.** 20 per level, then a "more" line (proposed)? Or no cut, since the browser (#34) will be the main way to explore?
4. **Threads.** Standard library threads only (proposed). The alternative is the `rayon` crate: less code, but a new dependency.

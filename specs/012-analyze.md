# 012: `molt analyze`

Status: Implemented (AN-01 to AN-17); Draft (AN-18 to AN-24, #35)
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
- **AN-06** The text output starts with the folder's total and path, then lists its children, largest first, then by name: size, the share of the total, and the name, with `/` after folders, `@` after symlinks, and `(mount)` after mount points. `--depth N` (default 1) also lists the children of folders, indented, down to N levels.
- **AN-07** Each level shows its 20 largest entries, then one line for the rest ("… 37 more, 1.2 GiB"). `--all` shows every entry.
- **AN-08** `--json` prints one object: the path, size, `partial`, and `children` down to `--depth`, each with name, kind (`dir`, `file`, `symlink`, `mount`, `other`), size and `partial`. `--all` doesn't apply; JSON has everything to that depth.
- **AN-09** `--dry-run` and the config's dry run are accepted and change nothing, as `analyze` changes nothing anyway (P-05). No dry-run notice is printed.

### Browser (#34)

The browser is a full-screen view over the same walk. It's written in molt: raw terminal mode through `rustix` termios and ANSI escape codes, with no terminal crate (Decisions).

- **AN-10** `molt analyze` opens the browser when stdin and stdout are both terminals, and `--json` and `--list` aren't given. Otherwise it prints the listing (AN-06), as now. `--list` forces the listing. Leaving the browser exits 0; `--depth` and `--all` apply to the listing only.
- **AN-11** While the walk runs, the screen shows a progress line (folders and bytes counted so far). `q` or Ctrl-C stops the walk and leaves.
- **AN-12** The screen shows the current folder's path and total, then its entries largest first: size (`+` if partial), share of the folder, a bar, and the name. One entry is selected.
- **AN-13** Keys: `j`/`k`/↓/↑ move, `g`/`G`/Home/End jump to the first or last entry, PgUp/PgDn move a screen, `l`/→/Enter open a folder, `h`/←/Backspace go up to the parent (never above the starting folder), `s` switches between size and name order, `r` measures the current folder again, `?` shows the keys, and `q` leaves.
- **AN-14** The browser changes nothing, except through AN-18 to AN-24.
- **AN-15** Leaving the browser in any way (`q`, Ctrl-C, an error, a panic) restores the terminal: cooked mode, the cursor shown, and the normal screen back.
- **AN-16** The screen redraws to fit when the terminal is resized. Names too long for the width are shortened in the middle with `…`, and a narrow terminal drops the bar before the name. A terminal that reports no size (0 × 0) is drawn at 80 × 24.
- **AN-17** Every folder of the walk is kept in memory, but a folder's files are read only when it's opened, so a home with millions of files fits in memory. A file whose hard-linked twin is elsewhere is shown at its own size there (AN-01 still holds for folder totals).

### Trashing picked items (#35)

The first and only way the browser changes anything. It goes through safe delete (P-01), in trash mode only, after the full dry run of exactly the picked items (P-14).

- **AN-18** `Space` marks or unmarks the selected entry and moves down. Marked entries show a `*`, and the bottom line shows how many are marked and their total size. Marks stay while moving between folders.
- **AN-19** Each mark records the entry as it is now (`Candidate::scan`: device, inode, type, mtime), so anything that changes before the trash step is skipped (SD-08, SD-10).
- **AN-20** `d` trashes the marked entries, or the selected entry if none are marked. It first runs a safe-delete dry run of exactly those entries, in trash mode, and shows its full result on screen: each entry, its size, and what would happen to it, including any refused (`protected`, `whitelisted`) or skipped, with the reason.
- **AN-21** Below that preview, molt asks "Move N items (SIZE) to the trash? [y/N]". Only `y` goes ahead. Any other key goes back to the browser with nothing changed and the marks kept.
- **AN-22** After `y`, one live safe-delete plan in trash mode runs over the same candidates. The screen then shows the result per entry (trashed, skipped or failed, with the reason), the marks are cleared, and the folders above the changed entries are measured again (as `r`).
- **AN-23** Nothing is ever deleted permanently from the browser: there is no key for it, and the plan's mode is always trash (SD-11). An entry the trash can't take (`no-trash`) is reported and kept.
- **AN-24** Every trashed entry is in the operation log (SD-15). A log that can't be opened stops the step before anything moves, and says so on screen.

## Test strategy

- Unit tests on a temp folder with known files: sizes match `st_blocks`, a hard link pair counts once, a symlink counts as itself, an unreadable folder (`chmod 000`) makes its parents partial, and the result is the same over many runs (AN-04).
- AN-02: a tmpfs mounted inside the temp folder in a user namespace, as 001's tests do; skipped where that isn't allowed.
- Smoke tests: text and `--json` shape, `--depth`, the 20-entry cut and `--all`, exit 3 on partial, exit 1 on a bad path.
- P-05: a snapshot of the temp folder is unchanged after a run.
- Browser: the key handling and the screen are pure functions from state and key to state, and from state and size to lines of text, so unit tests drive them without a terminal. One smoke test runs the browser under `script` (a pseudo-terminal), sends `j`, `l`, `h` and `q`, and checks the screen text and that the terminal is restored (the alternate-screen exit sequence is the last thing written).

## Open questions (#35)

1. **Keys.** `Space` to mark and `d` to trash (proposed, as in ncdu)?
2. **Trash only.** No permanent delete from the browser at all (proposed)? Someone who wants it can trash, then empty the trash.
3. **The question.** A plain `y` after the full preview (proposed), or type the item count, for big selections?
4. **Outside your home.** Browsing `/` or `/mnt/data` works, but trashing there needs a `.Trash-$uid` folder that molt can create on that filesystem (SD-11). Allow it as safe delete does (proposed), or only allow trashing inside `$HOME`?

## Decisions

- **The current directory by default**, like `du` and `ncdu`.
- **`--depth` 1 by default.**
- **20 entries per level**, then a "more" line; `--all` shows every entry.
- **The browser opens by default on a terminal**, as `ncdu` does; `--list` prints the listing instead.
- **Folders in memory, files read on opening** (AN-17).
- **Vim keys plus arrows** (AN-13).
- **The size is checked on every key and every 200 ms** (the read timeout, in tenths of a second), since catching `SIGWINCH` needs a crate or unsafe code.
- **No new dependency.** The parallel walk uses standard library threads, and the browser (#34) will drive the terminal through `rustix`. molt keeps full control of its code (maintainer decision).

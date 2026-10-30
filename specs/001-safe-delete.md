# 001: Safe delete

Status: Accepted
Prefix: SD

## Goal

A single module that removes paths found by a scan and cannot be tricked into removing anything else. Every command that frees space is built on it (P-01).

## Non-goals

- Deciding *what* to delete. That belongs to the rules engine (002) and to each command.
- Emptying the trash. That is a separate, explicit action.

## Model

A scan produces **candidates**. Each candidate holds the path plus the identity recorded during the scan: `(st_dev, st_ino, file type, mtime)`. A **plan** is a list of candidates and a mode (`permanent` or `trash`). Executing a plan produces a **report**: for each candidate, the outcome, the bytes freed, and the reason if it was skipped or failed.

## Requirements

### Refusals
- **SD-01** Given a candidate that is `/`, `$HOME`, an XDG base directory itself, a top-level system directory (`/usr`, `/etc`, `/boot`, `/var`, `/home`…), or any directory above one of these, when the plan runs, then it is refused with reason `protected`.
- **SD-02** Given a candidate inside a protected user location (`~/.ssh`, `~/.gnupg`, `~/.password-store`, `~/.local/share/keyrings`, a Firefox profile directory, or `key4.db`/`logins.json` inside one), or a directory above one of these, then it is refused with reason `protected`.
- **SD-03** Given a whitelisted path (P-03 config), then that path, everything inside it, and every directory above it are refused with reason `whitelisted`.
- **SD-04** Given a relative path, a path containing `..`, or a path that isn't valid UTF-8 after normalisation, then it is refused with reason `invalid-path`.

### Symlinks and mounts
- **SD-05** Given a candidate that is a symlink, then only the link itself is removed. The link's target is never read, followed, or removed.
- **SD-06** Given a directory candidate containing symlinks, then the links inside are removed as links, and nothing outside the candidate is touched.
- **SD-07** Given a directory candidate containing a mount point (a child whose `st_dev` differs from the candidate's), then nothing is deleted under that mount, the mount point directory is kept, and the outcome is `partial`. A candidate that is itself a mount point (its `st_dev` differs from its parent's) is skipped with reason `mount-point`. There is no flag to cross mounts.

### Races (TOCTOU)
- **SD-08** Given a candidate whose `(st_dev, st_ino, file type)` at execution differs from the scan, then it is skipped with reason `changed-since-scan`.
- **SD-09** Directory removal opens each level with `O_NOFOLLOW | O_DIRECTORY` and removes entries with `unlinkat` relative to the open directory handle. A path component swapped for a symlink during removal therefore cannot redirect deletion outside the candidate. A test must swap a directory for a symlink mid-removal and prove the target survives.
- **SD-10** Given a candidate whose mtime is newer than the scan, then it is skipped with reason `modified-since-scan`.

### Modes
- **SD-11** In `trash` mode, items are moved according to the FreeDesktop Trash spec:
  - Items on the same filesystem as `$XDG_DATA_HOME` go to `$XDG_DATA_HOME/Trash`.
  - Items on other filesystems go to `$topdir/.Trash-$uid`.
  - A matching `.trashinfo` file is written so the item can be restored with a normal file manager.
  - The `.trashinfo` file is created first with `O_EXCL`, which reserves the name. The item is then renamed in without replacing anything (`RENAME_NOREPLACE`). A taken name gets a numeric suffix (`name.2`, `name.3`…).
  - A trash directory that is a symlink, isn't a directory, or isn't owned by the user is not usable.
  - If no usable trash exists on the item's filesystem, the item is skipped with reason `no-trash`. It is never deleted permanently as a fallback.
  - A directory holding a mount is skipped with reason `mount-point`, so a mount is never carried into the trash (SD-07).
  - The outcome is `trashed`, with the bytes that emptying the trash would free (SD-13).
- **SD-12** In `--dry-run`, no mutating syscall is made. The report is identical to a real run's, except that outcomes read `would-remove`.

### Reporting
- **SD-13** Bytes freed are the sum of `st_blocks × 512` over the removed inodes. Each `(st_dev, st_ino)` with `st_nlink > 1` is counted once, and only when its last link inside the plan is removed.
- **SD-14** A failure on one candidate (EACCES, EBUSY, EROFS…) does not stop the plan. Each failure is reported with its errno. If any candidate failed, the exit code is 3 (P-13).
- **SD-15** Each removed or trashed candidate is appended to the operation log (P-09) with timestamp, mode, path, bytes, and outcome. The log entry is written after the removal succeeds.

### Privilege
- **SD-16** If the effective uid is 0 and the plan was not created by an explicit system step (P-04), the plan is refused as a whole before anything is touched.

## Test strategy

- Every test runs inside a temporary directory acting as `$HOME`, with all `XDG_*` variables pointed inside it. No test may resolve the real home directory.
- Tests use real filesystem operations, not mocks.
- Race tests (SD-09) use a hook that runs between directory levels and is available only in test builds.
- Property test: for random trees containing symlinks to outside targets, everything outside the candidate set has the same `(path, inode, size)` before and after.

## Decisions

- **Mounts are never crossed.** SD-07 has no `--cross-mounts` escape hatch. A mount inside a cleanable directory is almost always a mistake or a bind mount of real data.
- **Busy-file checks stay per-rule (P-08).** Safe delete does not scan `/proc/*/fd`. Doing that per file is too slow on trees like `node_modules`; rules that need it check their owning processes once, before planning.

# 009: `molt installer`

Status: Accepted
Prefix: IN

## Goal

`molt installer` finds the installers you downloaded and no longer need: `.deb`, `.rpm`, `.pkg.tar.zst`, `.AppImage`, `.iso`, `.run`. These are your files, not caches, so they go to the trash by default (000 Decisions) and you can get them back. It uses the same rules engine, preview, question and safe delete as `clean` (006).

## Non-goals

- Looking everywhere. Only the folders listed below, one level deep (RE-06's shallow globs).
- Telling whether an installer is still needed (installed, or the only copy of an ISO). molt can't know, so it previews, asks, and uses the trash.
- Making installers part of `clean`. A Downloads folder is user content; it's a separate command so `clean --yes` never touches it.

## Model

1. Start up (004). The installer rules are built-in rules (010's file) in the `installers` category. Startup sets them apart from the others, after `disabled_rules` (003) has dropped any, so `scan` and `clean` never see them.
2. Match them (002), keep only files older than the age limit (IN-03), and measure with a safe-delete dry run, in trash mode.
3. Preview, ask, act and report as `clean` does (CN-04 to CN-12), with `--permanent` to delete instead.

## Rules

One rule per file type, all `user-content` (so trash mode), in the `installers` category, shown under an "Installers" heading. Each matches directly in `~/Downloads` and `~/Desktop`.

| id | Files |
|---|---|
| `installer-deb` | `*.deb` |
| `installer-rpm` | `*.rpm` |
| `installer-pacman` | `*.pkg.tar.zst`, `*.pkg.tar.xz` |
| `installer-appimage` | `*.AppImage`, `*.appimage` |
| `installer-iso` | `*.iso` |
| `installer-run` | `*.run` |

## Requirements

- **IN-01** `molt installer` matches only its own rules; `clean` and `scan` never show or touch them. `--only`/`--skip` pick rules, with an unknown id a usage error (exit 2), as CN-02.
- **IN-02** Only regular files match: a directory is skipped with reason `not-a-file`, and a symlink with `symlink` (RE-10).
- **IN-03** A file modified in the last 14 days is skipped with reason `recently-changed` and counted in the preview, as it may be one you're about to use.
- **IN-04** The default mode is trash (SD-11). `--permanent` deletes instead, after the same preview and question.
- **IN-05** Everything else is `clean`'s: the full preview first (P-14), one question with no as the default, `--yes` needed without a terminal, `--dry-run` stops after the preview, the result and exit codes (CN-11), and the operation log.

## Test strategy

- Smoke tests on a temp home with installers of every type in `~/Downloads`, a folder named `x.iso`, a symlink `y.deb`, a new file, and a `.deb` in a subfolder (not matched: one level only).
- IN-01: `scan` on the same home doesn't list them.
- IN-04: the default run puts them in the temp home's trash; `--permanent` removes them.
- P-05: `--dry-run` and "no" leave a snapshot of the temp home unchanged.

## Decisions

- **14 days.** Newer files are left out, even though the trash would keep them.
- **`~/Downloads` and `~/Desktop` only.** Reading `$XDG_DOWNLOAD_DIR` from `user-dirs.dirs` (renamed or translated folders) can come later.
- **No `.img`.** VM disks share the extension.
- **An `installers` category**, shown as "Installers". User rules can use it too, and then they belong to `molt installer`.

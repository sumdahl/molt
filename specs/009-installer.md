# 009: `molt installer`

Status: Draft
Prefix: IN

## Goal

`molt installer` finds the installers you downloaded and no longer need: `.deb`, `.rpm`, `.pkg.tar.zst`, `.AppImage`, `.iso`, `.run`. These are your files, not caches, so they go to the trash by default (000 Decisions) and you can get them back. It uses the same rules engine, preview, question and safe delete as `clean` (006).

## Non-goals

- Looking everywhere. Only the folders listed below, one level deep (RE-06's shallow globs).
- Telling whether an installer is still needed (installed, or the only copy of an ISO). molt can't know, so it previews, asks, and uses the trash.
- Making installers part of `clean`. A Downloads folder is user content; it's a separate command so `clean --yes` never touches it.

## Model

1. Start up (004). Load the built-in installer rules (below), which aren't part of `clean`'s rules, and drop any listed in `disabled_rules` (003).
2. Match them (002), keep only files older than the age limit (IN-03), and measure with a safe-delete dry run, in trash mode.
3. Preview, ask, act and report as `clean` does (CN-04 to CN-12), with `--permanent` to delete instead.

## Rules

One rule per file type, all `user-content` (so trash mode), under an "Installers" heading (open question 4). Each matches in `~/Downloads` and `~/Desktop`, and in `$XDG_DOWNLOAD_DIR` if `user-dirs.dirs` sets it elsewhere (open question 2).

| id | Files |
|---|---|
| `installer-deb` | `*.deb` |
| `installer-rpm` | `*.rpm` |
| `installer-pacman` | `*.pkg.tar.zst`, `*.pkg.tar.xz` |
| `installer-appimage` | `*.AppImage`, `*.appimage` |
| `installer-iso` | `*.iso` (and `*.img`, open question 3) |
| `installer-run` | `*.run` |

## Requirements

- **IN-01** `molt installer` matches only its own rules; `clean` and `scan` never show or touch them. `--only`/`--skip` pick rules, with an unknown id a usage error (exit 2), as CN-02.
- **IN-02** Only regular files match: no directories, and no symlinks (RE-10).
- **IN-03** A file modified within the age limit is left out and counted in the preview ("left out: N newer than 14 days"), as it may be one you're about to use.
- **IN-04** The default mode is trash (SD-11). `--permanent` deletes instead, after the same preview and question.
- **IN-05** Everything else is `clean`'s: the full preview first (P-14), one question with no as the default, `--yes` needed without a terminal, `--dry-run` stops after the preview, the result and exit codes (CN-11), and the operation log.

## Test strategy

- Smoke tests on a temp home with installers of every type in `~/Downloads`, a folder named `x.iso`, a symlink `y.deb`, a new file, and a `.deb` in a subfolder (not matched: one level only).
- IN-01: `scan` on the same home doesn't list them.
- IN-04: the default run puts them in the temp home's trash; `--permanent` removes them.
- P-05: `--dry-run` and "no" leave a snapshot of the temp home unchanged.

## Open questions

1. **Age limit.** Leave out files changed in the last 14 days (proposed)? Or none, since everything goes to the trash anyway?
2. **Where.** `~/Downloads` and `~/Desktop` (proposed), plus `$XDG_DOWNLOAD_DIR` from `~/.config/user-dirs.dirs` when it points elsewhere? Reading that file is small but new; skipping it misses renamed Downloads folders (non-English desktops use `~/Téléchargements` and the like).
3. **`.img`.** Include disk images (`*.img`) with `.iso`? They're often installers, but also VM disks someone keeps on purpose. Proposed: leave `.img` out.
4. **Category.** Show them under a new "Installers" heading (proposed), which means adding an `installers` category that user rules could use too?

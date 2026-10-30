# 008: `onyx purge`

Status: Implemented
Prefix: PG

## Goal

`onyx purge` frees the build output of your own projects: `target`, `node_modules`, `.venv` and the like, which every rebuild or install brings back. It looks only under the folders you list in `purge_paths` (003), removes a build folder only when the project file that makes it sits next to it, leaves alone projects you're working on now, and goes through the same preview, question and safe delete as `clean` (006).

## Non-goals

- Caches outside projects. Those are rules (010).
- Guessing where projects live. With no `purge_paths`, purge has nothing to do and says how to set them.
- Running a project's own clean command (`cargo clean`, `gradle clean`). They need the toolchain, can run project code (`build.rs`, Gradle scripts) and do more than remove one folder. onyx removes the folder itself through safe delete (an exception to P-07, recorded below).
- Choosing single projects in a menu. That's phase 5 (#15); here `--only`/`--skip` pick kinds.

## Model

1. Start up (004). Read `purge_paths`.
2. **Find**: walk each root, without following symlinks or crossing filesystems, looking for a marker file next to a build folder (the table). A build folder is a candidate; onyx doesn't look inside it for more projects.
3. **Filter**: drop candidates in recently changed projects (PG-04), inside Git-tracked paths (PG-05), or whitelisted (SD-03).
4. **Preview** (P-14): a safe-delete dry run of every candidate, grouped by kind, with sizes, as `scan` shows rules.
5. **Ask** (P-06), then **act**: one live safe-delete plan, permanent (they're regenerable), over exactly what the preview measured (CN-09).
6. **Report** as `clean` does (CN-11).

## Kinds

| id | Build folder | Marker next to it |
|---|---|---|
| `cargo` | `target` | `Cargo.toml` |
| `node` | `node_modules` | `package.json` |
| `python-venv` | `.venv`, `venv` | `pyproject.toml`, `requirements.txt`, or `setup.py`, and the folder holds `pyvenv.cfg` |
| `gradle` | `build`, `.gradle` | `build.gradle` or `build.gradle.kts` |
| `maven` | `target` | `pom.xml` |
| `next` | `.next` | `package.json` and `next.config.*` |
| `zig` | `zig-cache`, `.zig-cache`, `zig-out` | `build.zig` |

## Requirements

### Finding
- **PG-01** Only folders under `purge_paths` are searched. With none set, `purge` prints how to set them (on stderr), shows an empty preview, and exits 0.
- **PG-02** A build folder is a candidate only when its kind's marker file is in the same directory. A folder named `build` or `target` with no marker is never a candidate.
- **PG-03** The walk never follows symlinks, never crosses filesystems, never descends into a candidate, and skips directories it can't read, reporting them (P-12) under the id `purge-paths`, as is a `purge_paths` entry that is a symlink. Hidden directories other than the build folders in the table aren't searched (`.git`, `.cache`).
- **PG-04** A project is left out when anything in its directory, other than its build folders, changed in the last 7 days (the newest mtime of the files directly in it and in `src/`, if present). The preview says how many projects were left out for this.
- **PG-05** A candidate holding a Git-tracked file is left out: onyx checks for a `.git` directory at the project or above it, and if found runs `git -c core.fsmonitor=false ls-files -z -- <folder>` in the project, with `GIT_OPTIONAL_LOCKS=0`, so a repository's config can't make onyx run its code or write to it. Any output, or a failure to run `git`, means leave it (P-02). Without `git` on `$PATH`, projects inside a Git repository are left out.
- **PG-06** A build folder inside another candidate's project is still found once: candidates never overlap.

### Preview, consent, acting
- **PG-07** `purge` follows CN-04 to CN-07 and CN-09 to CN-12, with kinds in place of rules: the full preview first (also with `--json` and `--yes`), one question with no as the default, `--yes` needed without a terminal, `--dry-run` stops after the preview, one live plan over the preview's candidates, the result and exit codes as CN-11, and every change logged.
- **PG-08** `--only <kind>…` and `--skip <kind>…` pick kinds; an unknown kind is a usage error (exit 2). `--trash` sends the candidates to the trash instead.

## Test strategy

- Smoke tests on a temp home with `purge_paths` pointing at a temp projects folder holding a few fake projects: a Cargo project with `target`, a `target` without `Cargo.toml`, a recently changed project (mtime set with `utimensat`), a project whose `target` holds a Git-tracked file (a real `git init` in the temp folder, skipped if `git` isn't installed), and a symlink to a project outside the root.
- P-05: `purge --dry-run` and "no" leave a snapshot of the temp home unchanged.

## Decisions

- **onyx deletes build folders itself, through safe delete.** Running `cargo clean` and the like would run project code (P-02 beats P-07 here).
- **The kinds are the table.** `dist`, a bare `build`, `__pycache__`, `.tox` and CMake builds are left out; any of them can come later with its own review.
- **Projects changed in the last 7 days are left out.** Not configurable yet.
- **A build folder holding any Git-tracked file is left out whole.**
- **No `purge_paths`, nothing to do.** onyx never guesses where projects live.

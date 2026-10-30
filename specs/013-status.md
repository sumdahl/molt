# 013: `molt status`

Status: Implemented (ST-01 to ST-08); #37 and #38 add more
Prefix: ST

## Goal

`molt status` shows the machine's health at a glance: CPU, memory, disks and stuck processes. Everything comes from `/proc` and `/sys`, read without root, and nothing is changed. This spec covers the one-shot snapshot (#36). Hardware readings and a health score (#37) and the live view (#38) add requirements here later.

## Non-goals

- Changing anything. `status` reads; fixing is `clean`, `system` and the planned `optimize` (#14).
- Per-process tables (`top`, `htop`). Only counts and the zombies.
- Guessing a value a machine doesn't report. A field that can't be read is left out (P-10).

## Requirements

- **ST-01** `molt status` reads only `/proc` and `/sys` (and `statvfs` for disks), never runs another program, never needs root, and changes nothing. `--dry-run` is accepted and makes no difference (P-05).
- **ST-02** **CPU**: the number of CPUs online, the load averages (1, 5, 15 minutes, `/proc/loadavg`), and how busy the CPUs were over a 200 ms sample (`/proc/stat`, the share of non-idle time), plus the uptime.
- **ST-03** **Memory**: total, available and used (`MemTotal − MemAvailable`) from `/proc/meminfo`, and swap total and used. No swap is shown as "none".
- **ST-04** **Disks**: each mounted filesystem backed by a device or a network share, from `/proc/self/mounts`: mount point, filesystem type, size, used, free for an unprivileged user (`statvfs` `f_bavail`), and the used share (rounded up in the text, as `df` does). Pseudo filesystems (`proc`, `sysfs`, `tmpfs`, `cgroup`, `overlay`, `squashfs` and the like) are left out, and so are bind mounts that repeat a device already listed (the one with the shortest mount point is kept). A filesystem that can't be measured (`statvfs` fails, e.g. a dead network share) is listed as unavailable, never waited on for more than 1 second.
- **ST-05** **Processes**: how many there are, how many are running, and every zombie (state `Z` in `/proc/<pid>/stat`) with its pid, name and parent pid, as zombies mean a parent that isn't reaping its children.
- **ST-06** The text output has one section per area, in the order above, with sizes as SC-04. A disk at 90 % or more is marked `(nearly full)`, and the line points at `molt analyze <mount point>`.
- **ST-07** `--json` prints one object with `cpu`, `memory`, `disks` and `processes`, sizes in bytes, shares as numbers from 0 to 100. A field the machine doesn't report is left out, not zero.
- **ST-08** The exit code is 0, also with nearly full disks: `status` reports, it doesn't judge (P-13 leaves 3 for partial actions).

## Test strategy

- The parsers take text (`/proc/stat`, `/proc/loadavg`, `/proc/meminfo`, `/proc/self/mounts`, `/proc/<pid>/stat`), so unit tests feed them fixtures, including missing fields, odd process names (`(a b) c)`), and mount paths with escaped spaces (`\040`).
- The disk filter has table tests: pseudo filesystems out, bind repeats out, the shortest mount point kept.
- A unit test spawns a child that exits without being waited for and checks it's listed as a zombie, then reaps it.
- Smoke tests on the real machine, read-only: exit 0, the four sections, the JSON shape, and a snapshot of the temp home unchanged.

## Decisions

- **A 200 ms CPU sample**, so "how busy" is measured, not guessed from the load.
- **Device-backed and network filesystems only**, without pseudo filesystems and bind repeats.
- **Nearly full at 90 %.**
- **Each disk is measured on its own thread, with a 1 second limit**, so a dead share can't hang `status`.

# 013: `onyx status`

Status: Implemented
Prefix: ST

## Goal

`onyx status` shows the machine's health at a glance: CPU, memory, disks and stuck processes. Everything comes from `/proc` and `/sys`, read without root, and nothing is changed. It covers the one-shot snapshot (#36), hardware readings and a health score (#37), and the live view (#38).

## Non-goals

- Changing anything. `status` reads; fixing is `clean`, `system` and the planned `optimize` (#14).
- Per-process tables (`top`, `htop`). Only counts and the zombies.
- Guessing a value a machine doesn't report. A field that can't be read is left out (P-10).

## Requirements

- **ST-01** `onyx status` reads only `/proc` and `/sys` (and `statvfs` for disks), never runs another program, never needs root, and changes nothing. `--dry-run` is accepted and makes no difference (P-05).
- **ST-02** **CPU**: the number of CPUs online, the load averages (1, 5, 15 minutes, `/proc/loadavg`), and how busy the CPUs were over a 200 ms sample (`/proc/stat`, the share of non-idle time), plus the uptime.
- **ST-03** **Memory**: total, available and used (`MemTotal − MemAvailable`) from `/proc/meminfo`, and swap total and used. No swap is shown as "none".
- **ST-04** **Disks**: each mounted filesystem backed by a device or a network share, from `/proc/self/mounts`: mount point, filesystem type, size, used, free for an unprivileged user (`statvfs` `f_bavail`), and the used share (rounded up in the text, as `df` does). Pseudo filesystems (`proc`, `sysfs`, `tmpfs`, `cgroup`, `overlay`, `squashfs` and the like) are left out, and so are bind mounts that repeat a device already listed (the one with the shortest mount point is kept). A filesystem that can't be measured (`statvfs` fails, e.g. a dead network share) is listed as unavailable, never waited on for more than 1 second.
- **ST-05** **Processes**: how many there are, how many are running, and every zombie (state `Z` in `/proc/<pid>/stat`) with its pid, name and parent pid, as zombies mean a parent that isn't reaping its children.
- **ST-06** The text output has one section per area, in the order above, with sizes as SC-04. A disk at 90 % or more is marked `(nearly full)`, and the line points at `onyx analyze <mount point>`.
- **ST-07** `--json` prints one object with `cpu`, `memory`, `disks` and `processes`, sizes in bytes, shares as numbers from 0 to 100. A field the machine doesn't report is left out, not zero.
- **ST-08** The exit code is 0, also with nearly full disks: `status` reports, it doesn't judge (P-13 leaves 3 for partial actions).

### Hardware and the health score (#37)

- **ST-09** **Rates**: disk reads and writes per second for each whole disk (`/proc/diskstats`, sectors × 512, partitions and `loop`/`ram`/`zram` devices left out) and network received and sent per second for each interface except `lo` (`/proc/net/dev`), over the same 200 ms sample as the CPU. The text leaves out interfaces with no traffic in the sample and says how many; `--json` has them all.
- **ST-10** **Temperatures**: from `/sys/class/hwmon/*/temp*_input` (millidegrees), grouped by sensor `name`, with each reading's `temp*_label` if there is one. The text shows the hottest reading per sensor; `--json` has them all. A reading above its `temp*_crit` (or `temp*_max`, if there's no critical one) is marked `(hot)`.
- **ST-11** **Fans**: each `fan*_input` (RPM) with its sensor name. A fan at 0 RPM is shown as stopped, not as a fault: many fans stop when cool.
- **ST-12** **Battery**: for each `/sys/class/power_supply/*` of `type` `Battery`, the charge (`capacity`, %), the `status` (Charging, Discharging, Full, Not charging), and its health as `energy_full` / `energy_full_design` (or the `charge_*` pair) where both exist.
- **ST-13** **Health score**: a whole number from 0 to 100, written down in full here so it's never a mystery. It starts at 100 and loses points:
  - each disk at 90 % or more: −15; at 80 % or more: −5,
  - memory available under 10 % of total: −15; under 20 %: −5,
  - swap used over 50 %: −5,
  - load over 2 × the CPU count (1-minute load): −10,
  - any reading marked `(hot)`: −15,
  - each zombie: −2, at most −10,
  - battery health under 60 %: −10,
  and never drops below 0. The text shows the score and every reason that cost points ("−15: / is 92 % full"); `--json` has `score` and `reasons`.
- **ST-14** Anything the machine doesn't have (no battery, no fans, no hwmon) is left out of the text and the JSON, and costs no points.

### Live view (#38)

- **ST-15** `onyx status --watch` shows the same report full screen and refreshes it every 2 seconds, until `q` or Ctrl-C. `--interval <seconds>` (1 to 60) changes the period. The CPU, disk and network rates are measured over the whole period instead of 200 ms, so they're steadier.
- **ST-16** It uses the same terminal code as the `analyze` browser (AN-15, AN-16): raw mode through `rustix`, the alternate screen, the terminal always restored, redrawn to fit the window, and long lines cut to its width.
- **ST-17** Keys: `q` or Ctrl-C quit, `p` pauses and resumes (the screen says "paused"), and `r` refreshes now.
- **ST-18** `--watch` needs a terminal on stdin and stdout. Without one, or with `--json`, it's a usage error (exit 2) naming why; `onyx status --json` stays the way to poll from a script.
- **ST-19** The view only reads, like the snapshot (ST-01).

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
- **The health score is ST-13's formula, as listed.** Simple on purpose, so anyone can check a score by hand.
- **The text shows the hottest reading per sensor**; `--json` has every reading.
- **Temperatures in °C.**
- **`--watch` refreshes every 2 seconds**, with `--interval` from 1 to 60.
- **Rates in the live view are over the whole period**, so they're steadier.
- **`q`/Ctrl-C quit, `p` pauses, `r` refreshes now.**
- **One shared terminal module** for the `analyze` browser and the live view.

# 010: Built-in rules

Status: Accepted
Prefix: BR

## Goal

The first set of rules molt ships with: the caches that actually fill a Linux developer's disk. Each one is data the owning program rebuilds by itself, so removing it costs a re-download or a rebuild, never the user's work. The list is written down and reviewed here, because a wrong path in it deletes the wrong data on every machine that runs molt.

## Non-goals

- System-level data (pacman cache, journal, coredumps, Docker). That is the system step (007, #7).
- Project build folders (`target`, `node_modules`). That is `purge` (008, #8).
- Installed language versions (`mise prune`, `rustup toolchain`). They are not caches, and removing them changes what runs.
- Anything specific to one machine, such as a project's VM image cache. Users add those as their own rules (RE-01).

## Requirements

- **BR-01** Every built-in rule has a test that builds its directory layout in a fake home, including a sibling that must not match, and checks that the rule matches exactly the intended paths.
- **BR-02** A rule whose data the owning program keeps open while running names that program's processes (P-08).
- **BR-03** A rule names its owning tool's cleanup command (P-07) when the tool has one that cleans exactly that data. It doesn't when the command does more: `yay -Sc` also cleans the system pacman cache with `sudo`.
- **BR-04** A rule with a cleanup command requires that tool (RE-08), so it's inactive where the tool isn't installed.
- **BR-05** A rule only matches inside a cache-like directory: never a whole app config directory, and never anything holding settings, history, passwords or sessions.

## The rules

All are `regenerable`, so they're deleted permanently (000 Decisions). `$C` is `$XDG_CACHE_HOME`, `$D` is `$XDG_DATA_HOME`, `$K` is `$XDG_CONFIG_HOME`.

### Developer tools

| id | Paths | Cleanup command | Processes |
|---|---|---|---|
| `npm-cache` | `~/.npm/_cacache` | `npm cache clean --force` | `npm` |
| `pnpm-store` | `$D/pnpm/store` | `pnpm store prune` | `pnpm` |
| `yarn-cache` | `$C/yarn` | `yarn cache clean` | `yarn` |
| `bun-cache` | `~/.bun/install/cache` | `bun pm cache rm` | `bun` |
| `cargo-registry` | `~/.cargo/registry/cache`, `~/.cargo/registry/src` | none (stable cargo has no gc) | `cargo` |
| `cargo-git` | `~/.cargo/git/checkouts` | none | `cargo` |
| `go-build-cache` | `$C/go-build` | `go clean -cache` | `go` |
| `go-module-cache` | `~/go/pkg/mod` | `go clean -modcache` (its files are read-only, so the tool must do it) | `go` |
| `gradle-caches` | `~/.gradle/caches`, `~/.gradle/wrapper/dists` | none | `java` (the Gradle daemon) |
| `maven-repository` | `~/.m2/repository` | none | `java` |
| `uv-cache` | `$C/uv` | `uv cache clean` | `uv` |
| `pip-cache` | `$C/pip` | none (`pip` vs `pip3` varies) | `pip`, `pip3` |
| `mise-cache` | `$C/mise` | `mise cache clear` | `mise` |
| `yay-cache` | `$C/yay` | none (`yay -Sc` also touches the system) | `yay` |
| `paru-cache` | `$C/paru/clone` | none | `paru` |
| `jetbrains-cache` | `$C/JetBrains/*` | none | `java` |

### Browsers

| id | Paths | Processes |
|---|---|---|
| `firefox-cache` | `$C/mozilla/firefox/*/cache2` | `firefox` |
| `chrome-cache` | `$C/google-chrome/*/Cache`, `$C/google-chrome/*/Code Cache` | `chrome` |
| `chromium-cache` | `$C/chromium/*/Cache`, `$C/chromium/*/Code Cache` | `chromium` |
| `brave-cache` | `$C/BraveSoftware/Brave-Browser/*/Cache`, `…/*/Code Cache` | `brave` |

Only the cache folders, never the profile: bookmarks, passwords and sessions live in `~/.mozilla` and `$K/…`, which SD-02 protects as well.

### Apps

| id | Paths | Processes |
|---|---|---|
| `spotify-cache` | `$C/spotify` | `spotify` |
| `discord-cache` | `$K/discord/Cache`, `$K/discord/Code Cache` | `Discord`, `discord` |
| `slack-cache` | `$K/Slack/Cache`, `$K/Slack/Code Cache` | `slack` |

### System (your files)

| id | Paths | Processes |
|---|---|---|
| `thumbnails` | `$C/thumbnails` | none |

## Test strategy

- One table-driven test (BR-01) that, for each rule, lays out matching and non-matching paths in a fake home and asserts the matches.
- A test that every rule with `clean_with` also has `requires` naming its first word (BR-04).

## Decisions

- **No trash rule.** Emptying the trash deletes what the user only set aside; it will be a separate, explicit action later.
- **`java` blocks Gradle, Maven and JetBrains.** Skipping too often is safe; deleting under a running build or IDE is not.
- **Maven is included.** `clean` lets the user untick it, and the whitelist can protect it.

# 019: The `molt` menu

Status: Accepted (#60)
Prefix: MN

## Goal

A bare `molt` on a terminal opens a small full-screen menu of what molt can do, under the banner (016). Picking an entry runs that command exactly as typing it would: its dry run first, its preview, its own question (P-14). Scripts and pipes keep getting `molt scan`.

## Non-goals

- A second way to delete. The menu only starts commands; each one runs its own code path, flags and question.
- Choosing options inside the menu (`--only`, `--permanent`, paths). The menu runs each command with its defaults; the command line is for the rest.
- A dependency. It uses molt's own terminal code (`src/terminal.rs`, AN-15, AN-16).

## The menu

```text
<banner>

> Scan        what could be freed (changes nothing)
  Clean       caches and other regenerable data
  Purge       build folders of your projects
  Installers  old installers in Downloads and Desktop
  Uninstall   an app and what it left behind
  Analyze     what takes the space in your home
  Status      the machine's health, live
  System      pacman cache, journal, coredumps (sudo)
  Optimize    unneeded packages and upkeep (sudo)
  History     what molt has removed
  Quit

↑↓/jk move  ·  Enter run  ·  q quit
```

## Requirements

- **MN-01** A bare `molt`, with stdin and stdout both terminals and no `--json`, opens the menu. Otherwise a bare `molt` is `molt scan`, as before. This replaces BN-02's "bare `molt` run" with "the menu".
- **MN-02** The menu is drawn full screen with molt's terminal code: raw mode, the alternate screen, redrawn to fit, and the terminal always restored (AN-15, AN-16). `j`/`k` and the arrow keys move, Enter runs, `q`, Esc and Ctrl-C quit with exit 0.
- **MN-03** Running an entry leaves the full screen first, then runs exactly `molt <command>` with the global flags the menu was started with (`--dry-run`, `--live`, `--yes`), through the same code as the command line: the same preview, dry run and question. `Uninstall` first asks for the app's name on a normal line; an empty name goes back to the menu, and a bad one gets UN-01's error. `Analyze` opens the browser on `~`; `Status` opens `status --watch`.
- **MN-04** After the command ends, molt prints its exit code if it wasn't 0 and waits for Enter, then shows the menu again. The menu's own exit code is 0.
- **MN-05** The menu itself changes nothing and runs nothing until Enter.

## Test strategy

- Unit tests: the key decoder (arrows, `j`/`k`, Enter, `q`, Esc, Ctrl-C), moving past the ends, and each entry's argument list (`scan`, `clean`, … `analyze ~`, `status --watch`) with the global flags passed on.
- Smoke tests: piped bare `molt` is still a scan with no menu (MN-01); a pty bare `molt` shows the menu and `q` restores the terminal; in a temp home, picking `Clean` with `--dry-run` shows clean's preview and changes nothing (P-05); picking `Uninstall` with an empty name goes back.

## Decisions

- **Commands run in molt itself**, through the same parse and dispatch as the command line, so the menu can't drift from what the flags do.
- **No options in the menu.** Every option has a safe default; a menu of options would be a second interface to keep correct.
- **Status runs live**; a snapshot would vanish behind the "press Enter" line.

# 016: The molt banner

Status: Draft (#52)
Prefix: BN

## Goal

An ASCII-art "molt" greets you on a terminal, at the top of `molt --help` and of a bare `molt` run, so molt looks like a finished tool. Scripts and pipes see exactly what they see today.

## Non-goals

- A banner on every command. `clean`, `system` and the rest start with their preview; nothing goes above it.
- Animation, themes or config.
- A dependency. The banner is a constant in molt.

## The banner

```text
                 _ _
 _ __ ___   ___ | | |_
| '_ ` _ \ / _ \| | __|
| | | | | | (_) | | |_
|_| |_| |_|\___/|_|\__|
  a careful disk cleaner for Linux
```

## Requirements

- **BN-01** The banner is the six lines above: printable ASCII only, at most 40 columns wide, followed by one empty line.
- **BN-02** It's printed at the top of `molt --help` and `molt help`, and of a bare `molt` run (the default `molt scan`), when stdout is a terminal.
- **BN-03** It's never printed when stdout isn't a terminal, with `--json`, with `--version`, for any subcommand named on the command line (`molt scan` included), or on stderr. That output is byte for byte what it was before.
- **BN-04** On a terminal the art is drawn in green and the tagline dimmed, then the colour is reset. With `NO_COLOR` set to anything but the empty string (no-color.org), or `TERM=dumb`, it's plain text with no escape codes.

## Test strategy

- Unit tests: BN-01's size and character limits, and BN-04's coloured and plain forms.
- Smoke tests: piped `molt --help`, `molt` and `molt --version` don't contain the art (BN-03), and `tests/smoke.rs`'s existing checks still pass unchanged.
- A pty smoke test: `molt --help` shows the art above the usage line, coloured, and with `NO_COLOR=1` without any escape code.

## Decisions

- **Help and bare `molt` only.** Those are where someone meets molt. A banner above a deletion preview would push the important part down.
- **Green**, for the fresh skin a molt leaves. One colour keeps it readable on light and dark terminals.
- **No off switch.** Piping or `NO_COLOR` already covers the cases where it's unwanted.

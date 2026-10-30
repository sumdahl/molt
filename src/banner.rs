//! The onyx banner (specs/016-banner.md): an ASCII-art greeting on a terminal only.

use std::io::IsTerminal;

#[cfg(test)]
mod tests;

/// BN-01
const ART: &str = r"  ___  _ __  _   ___  __
 / _ \| '_ \| | | \ \/ /
| (_) | | | | |_| |>  <
 \___/|_| |_|\__, /_/\_\
             |___/";

const TAGLINE: &str = "  a careful disk cleaner for Linux";

/// BN-04
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Colour {
    On,
    Off,
}

impl Colour {
    /// no-color.org: any non-empty `NO_COLOR` turns colour off, as does `TERM=dumb`.
    fn from_env(no_color: Option<&str>, term: Option<&str>) -> Self {
        if no_color.is_some_and(|value| !value.is_empty()) || term == Some("dumb") {
            Self::Off
        } else {
            Self::On
        }
    }
}

fn banner(colour: Colour) -> String {
    match colour {
        Colour::On => format!("\x1b[32m{ART}\n\x1b[0m\x1b[2m{TAGLINE}\x1b[0m\n\n"),
        Colour::Off => format!("{ART}\n{TAGLINE}\n\n"),
    }
}

/// BN-02, BN-03: the banner when stdout is a terminal, else nothing.
pub fn for_terminal() -> Option<String> {
    if !std::io::stdout().is_terminal() {
        return None;
    }
    let var = |name| std::env::var(name).ok();
    let colour = Colour::from_env(var("NO_COLOR").as_deref(), var("TERM").as_deref());
    Some(banner(colour))
}

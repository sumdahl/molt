//! `molt uninstall` (specs/014-uninstall.md): what an app left in your home, as rules
//! built for this one run so it goes through `clean`'s whole path.

use std::fmt;

use crate::rules::{Category, Kind, Pattern, Rule, RuleId};

pub mod package;
#[cfg(test)]
mod tests;

/// UN-03: folders many apps share, so no app's name.
const SHARED: [&str; 20] = [
    "applications",
    "autostart",
    "icons",
    "fonts",
    "mime",
    "themes",
    "sounds",
    "Trash",
    "systemd",
    "environment.d",
    "dbus-1",
    "keyrings",
    "flatpak",
    "config",
    "cache",
    "local",
    "ssh",
    "gnupg",
    "pki",
    // Its log is the record of what molt removed.
    "molt",
];

/// An app's name, checked by UN-01 and UN-03, so it names exactly one path per location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppName(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Invalid,
    Shared,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => {
                "an app name is 1 to 100 ASCII letters, digits, `.`, `_`, `+` or `-`, \
                 not starting with `.` or `-`"
            }
            Self::Shared => "that name is a folder many apps share, not one app's",
        })
    }
}

impl std::error::Error for NameError {}

impl TryFrom<&str> for AppName {
    type Error = NameError;

    fn try_from(name: &str) -> Result<Self, NameError> {
        // UN-01: no `/`, `..` or glob character can reach a path.
        let valid = (1..=100).contains(&name.len())
            && !name.starts_with(['.', '-'])
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b));
        if !valid {
            Err(NameError::Invalid)
        } else if SHARED.contains(&name) {
            Err(NameError::Shared)
        } else {
            Ok(Self(name.to_owned()))
        }
    }
}

impl fmt::Display for AppName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// UN-02: the paths of one location, each `before`, the name, then `after`.
struct Location {
    id: &'static str,
    what: &'static str,
    paths: &'static [(&'static str, &'static str)],
    kind: Kind,
}

const LOCATIONS: [Location; 11] = [
    Location {
        id: "uninstall-config",
        what: "settings",
        paths: &[("$XDG_CONFIG_HOME/", "")],
        kind: Kind::UserContent,
    },
    Location {
        id: "uninstall-data",
        what: "data",
        paths: &[("$XDG_DATA_HOME/", "")],
        kind: Kind::UserContent,
    },
    Location {
        id: "uninstall-state",
        what: "state",
        paths: &[("$XDG_STATE_HOME/", "")],
        kind: Kind::UserContent,
    },
    // UN-05: deleted like every regenerable rule.
    Location {
        id: "uninstall-cache",
        what: "cache",
        paths: &[("$XDG_CACHE_HOME/", "")],
        kind: Kind::Regenerable,
    },
    Location {
        id: "uninstall-dotfolder",
        what: "dot-folder",
        paths: &[("~/.", "")],
        kind: Kind::UserContent,
    },
    Location {
        id: "uninstall-menu-entry",
        what: "menu entry",
        paths: &[("$XDG_DATA_HOME/applications/", ".desktop")],
        kind: Kind::UserContent,
    },
    Location {
        id: "uninstall-autostart",
        what: "autostart entry",
        paths: &[("$XDG_CONFIG_HOME/autostart/", ".desktop")],
        kind: Kind::UserContent,
    },
    // UN-19
    Location {
        id: "uninstall-flatpak-data",
        what: "flatpak data",
        paths: &[("~/.var/app/", "")],
        kind: Kind::UserContent,
    },
    Location {
        id: "uninstall-snap-data",
        what: "snap data",
        paths: &[("~/snap/", "")],
        kind: Kind::UserContent,
    },
    // UN-20: exact names only, so no versioned file names.
    Location {
        id: "uninstall-appimage",
        what: "AppImage",
        paths: &[
            ("~/Applications/", ".AppImage"),
            ("~/Applications/", ".appimage"),
            ("~/.local/bin/", ".AppImage"),
            ("~/.local/bin/", ".appimage"),
        ],
        kind: Kind::UserContent,
    },
    Location {
        id: "uninstall-icon",
        what: "icons",
        paths: &[
            ("$XDG_DATA_HOME/icons/hicolor/*/apps/", ".png"),
            ("$XDG_DATA_HOME/icons/hicolor/*/apps/", ".svg"),
        ],
        kind: Kind::UserContent,
    },
];

/// One rule per location for `app`, with `app` as the process that owns it (P-08).
pub fn rules(app: &AppName) -> Result<Vec<Rule>, String> {
    LOCATIONS
        .iter()
        .map(|location| {
            Ok(Rule {
                id: RuleId::builtin(location.id),
                name: format!("{app} {}", location.what),
                category: Category::Apps,
                kind: location.kind,
                paths: location
                    .paths
                    .iter()
                    .map(|(before, after)| Pattern::try_from(format!("{before}{app}{after}")))
                    .collect::<Result<_, _>>()?,
                clean_with: Vec::new(),
                processes: vec![app.to_string()],
                requires: Vec::new(),
            })
        })
        .collect()
}

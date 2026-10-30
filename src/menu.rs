//! The `molt` menu (specs/019-menu.md): a full-screen list of commands. It only picks
//! one; the command line's own code runs it (MN-03).

use std::io;

use crate::terminal::Screen;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Scan,
    Clean,
    Purge,
    Installers,
    Uninstall,
    Analyze,
    Status,
    System,
    Optimize,
    History,
    Quit,
}

impl Entry {
    const ALL: [Self; 11] = [
        Self::Scan,
        Self::Clean,
        Self::Purge,
        Self::Installers,
        Self::Uninstall,
        Self::Analyze,
        Self::Status,
        Self::System,
        Self::Optimize,
        Self::History,
        Self::Quit,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Scan => "Scan",
            Self::Clean => "Clean",
            Self::Purge => "Purge",
            Self::Installers => "Installers",
            Self::Uninstall => "Uninstall",
            Self::Analyze => "Analyze",
            Self::Status => "Status",
            Self::System => "System",
            Self::Optimize => "Optimize",
            Self::History => "History",
            Self::Quit => "Quit",
        }
    }

    fn about(self) -> &'static str {
        match self {
            Self::Scan => "what could be freed (changes nothing)",
            Self::Clean => "caches and other regenerable data",
            Self::Purge => "build folders of your projects",
            Self::Installers => "old installers in Downloads and Desktop",
            Self::Uninstall => "an app and what it left behind",
            Self::Analyze => "what takes the space in your home",
            Self::Status => "the machine's health, live",
            Self::System => "pacman cache, journal, coredumps (sudo)",
            Self::Optimize => "unneeded packages and upkeep (sudo)",
            Self::History => "what molt has removed",
            Self::Quit => "",
        }
    }

    /// MN-03: the command it runs. `uninstall` gets the app's name and `analyze` the
    /// home folder after these.
    pub fn args(self) -> &'static [&'static str] {
        match self {
            Self::Scan => &["scan"],
            Self::Clean => &["clean"],
            Self::Purge => &["purge"],
            Self::Installers => &["installer"],
            Self::Uninstall => &["uninstall"],
            Self::Analyze => &["analyze"],
            Self::Status => &["status", "--watch"],
            Self::System => &["system"],
            Self::Optimize => &["optimize"],
            Self::History => &["history"],
            Self::Quit => &[],
        }
    }
}

/// MN-02
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Up,
    Down,
    Run,
    Quit,
}

fn keys(bytes: &[u8]) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut rest = bytes;
    while let Some((&first, tail)) = rest.split_first() {
        rest = tail;
        let key = match first {
            b'k' => Key::Up,
            b'j' => Key::Down,
            b'\r' | b'\n' => Key::Run,
            // Raw mode turns Ctrl-C into a byte.
            b'q' | 0x03 => Key::Quit,
            0x1b if rest.is_empty() => Key::Quit,
            0x1b => {
                let arrow = [
                    (b"[A", Key::Up),
                    (b"OA", Key::Up),
                    (b"[B", Key::Down),
                    (b"OB", Key::Down),
                ]
                .into_iter()
                .find(|(sequence, _)| rest.starts_with(*sequence));
                // An unknown sequence is skipped whole, so its bytes aren't read as keys.
                let len = arrow.map_or_else(
                    || {
                        rest.iter()
                            .position(|b| b.is_ascii_alphabetic() || *b == b'~')
                            .map_or(rest.len(), |at| at + 1)
                    },
                    |(sequence, _)| sequence.len(),
                );
                rest = &rest[len..];
                match arrow {
                    Some((_, key)) => key,
                    None => continue,
                }
            }
            _ => continue,
        };
        keys.push(key);
    }
    keys
}

fn step(selected: usize, key: Key) -> usize {
    match key {
        Key::Up => selected.saturating_sub(1),
        Key::Down => (selected + 1).min(Entry::ALL.len() - 1),
        Key::Run | Key::Quit => selected,
    }
}

const FOOTER: &str = "↑↓/jk move  ·  Enter run  ·  q quit";

/// MN-02: the banner when there's room, the entries with `>` at the choice, the keys.
fn lines(banner: &[&str], selected: usize, width: usize, height: usize) -> Vec<String> {
    let cut = |line: String| line.chars().take(width).collect::<String>();
    let menu_height = Entry::ALL.len() + 2;
    let mut lines = Vec::new();
    if banner.len() + 1 + menu_height <= height {
        lines.extend(banner.iter().map(|line| (*line).to_owned()));
        lines.push(String::new());
    }
    for (index, entry) in Entry::ALL.iter().enumerate() {
        let mark = if index == selected { '>' } else { ' ' };
        let line = format!("{mark} {:<10}  {}", entry.label(), entry.about());
        lines.push(cut(line.trim_end().to_owned()));
    }
    lines.push(String::new());
    lines.push(cut(FOOTER.to_owned()));
    lines
}

/// MN-02, MN-05: shows the menu until an entry is picked, starting at `selected` and
/// leaving it on the choice. `None` to quit. The screen is restored before returning.
pub fn pick(banner: &str, selected: &mut usize) -> io::Result<Option<Entry>> {
    let screen = Screen::enter()?;
    // The banner's own escapes would count as columns, so it shows only when it fits.
    let banner: Vec<&str> = banner.lines().filter(|line| !line.is_empty()).collect();
    loop {
        let (width, height) = screen.size();
        let banner: &[&str] = if width >= 40 { &banner } else { &[] };
        screen.draw(&lines(banner, *selected, width, height))?;
        for key in keys(&screen.read_bytes()?) {
            match key {
                Key::Quit => return Ok(None),
                Key::Run => {
                    let entry = Entry::ALL[*selected];
                    return Ok((entry != Entry::Quit).then_some(entry));
                }
                Key::Up | Key::Down => *selected = step(*selected, key),
            }
        }
    }
}

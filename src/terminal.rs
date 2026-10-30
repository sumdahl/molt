//! A full-screen terminal for the `analyze` browser and `status --watch` (AN-15, AN-16,
//! ST-16): raw mode through `rustix` termios and ANSI escapes, written in onyx, with no
//! terminal crate (012 Decisions).

use std::io::{self, Write};
use std::os::fd::AsFd;

use rustix::termios::{self, OptionalActions, SpecialCodeIndex, Termios};

/// The terminal in raw mode on the alternate screen. Dropping it puts everything back,
/// whether onyx leaves by `q`, an error or a panic (AN-15).
pub(crate) struct Screen {
    saved: Termios,
}

impl Screen {
    pub(crate) fn enter() -> io::Result<Self> {
        let stdin = io::stdin();
        let saved = termios::tcgetattr(stdin.as_fd())?;
        let mut raw = saved.clone();
        raw.make_raw();
        // A read returns after 200 ms with nothing, so the screen keeps up while idle.
        raw.special_codes[SpecialCodeIndex::VMIN] = 0;
        raw.special_codes[SpecialCodeIndex::VTIME] = 2;
        termios::tcsetattr(stdin.as_fd(), OptionalActions::Now, &raw)?;
        let screen = Self { saved };
        // The alternate screen, and no cursor.
        write_out("\x1b[?1049h\x1b[?25l")?;
        Ok(screen)
    }

    /// Columns and rows, or 80 × 24 if the terminal won't say (a 0 × 0 pty).
    pub(crate) fn size(&self) -> (usize, usize) {
        match termios::tcgetwinsize(io::stdout().as_fd()) {
            Ok(size) if size.ws_col > 0 && size.ws_row > 0 => (
                usize::from(size.ws_col.max(20)),
                usize::from(size.ws_row.max(5)),
            ),
            _ => (80, 24),
        }
    }

    /// What was typed, after at most 200 ms; empty if nothing was.
    pub(crate) fn read_bytes(&self) -> io::Result<Vec<u8>> {
        let mut buf = [0u8; 64];
        match rustix::io::read(io::stdin().as_fd(), &mut buf) {
            Ok(len) => Ok(buf[..len].to_vec()),
            Err(rustix::io::Errno::INTR) => Ok(Vec::new()),
            Err(errno) => Err(errno.into()),
        }
    }

    /// Waits for a key and gives its first byte.
    pub(crate) fn wait_byte(&self) -> io::Result<u8> {
        loop {
            if let Some(&first) = self.read_bytes()?.first() {
                return Ok(first);
            }
        }
    }

    /// Draws `lines` from the top, clearing what's left of each line and below.
    pub(crate) fn draw(&self, lines: &[String]) -> io::Result<()> {
        let mut frame = String::from("\x1b[H");
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                frame.push_str("\r\n");
            }
            frame.push_str(line);
            frame.push_str("\x1b[K");
        }
        frame.push_str("\x1b[J");
        write_out(&frame)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        // Best effort: there is nowhere left to report a failure to.
        let _ = termios::tcsetattr(io::stdin().as_fd(), OptionalActions::Now, &self.saved);
        let _ = write_out("\x1b[?25h\x1b[?1049l");
    }
}

fn write_out(text: &str) -> io::Result<()> {
    let mut out = io::stdout().lock();
    out.write_all(text.as_bytes())?;
    out.flush()
}

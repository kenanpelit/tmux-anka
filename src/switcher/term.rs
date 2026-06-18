//! Raw-mode terminal I/O for the switcher (dependency-free, via `stty`).

use std::io::Write;
use std::process::Command;

use crate::switcher::Key;

/// Decode the next key from a byte buffer.
///
/// - `None` → the buffer ends mid-escape-sequence; read more bytes and retry.
/// - `Some((None, n))` → consumed `n` bytes that map to no key (ignore them).
/// - `Some((Some(key), n))` → a key, consuming `n` bytes.
pub fn parse_key(buf: &[u8]) -> Option<(Option<Key>, usize)> {
    let b0 = *buf.first()?;
    match b0 {
        b'\r' | b'\n' => Some((Some(Key::Enter), 1)),
        b'\t' => Some((Some(Key::Tab), 1)),
        0x7f | 0x08 => Some((Some(Key::Backspace), 1)),
        0x0e => Some((Some(Key::Down), 1)),   // ^N (next, fzf-style)
        0x10 => Some((Some(Key::Up), 1)),     // ^P (prev, fzf-style)
        0x12 => Some((Some(Key::Rename), 1)), // ^R
        0x18 => Some((Some(Key::Delete), 1)), // ^X
        0x03 => Some((Some(Key::Cancel), 1)), // ^C
        0x1b => {
            if buf.len() == 1 {
                return Some((Some(Key::Cancel), 1)); // lone ESC
            }
            if buf[1] == b'[' || buf[1] == b'O' {
                if buf.len() < 3 {
                    return None; // need the final byte
                }
                return match buf[2] {
                    b'A' => Some((Some(Key::Up), 3)),
                    b'B' => Some((Some(Key::Down), 3)),
                    _ => Some((None, 3)), // ignore other CSI/SS3 sequences
                };
            }
            Some((Some(Key::Cancel), 2)) // ESC + other → treat as cancel
        }
        b'1'..=b'9' => Some((Some(Key::Digit((b0 - b'0') as usize)), 1)),
        0x01..=0x1a => Some((Some(Key::Ctrl((b0 + 0x60) as char)), 1)),
        0x20..=0x7e => Some((Some(Key::Char(b0 as char)), 1)),
        0x80.. => {
            // UTF-8 multibyte: decode just the first char.
            let s = std::str::from_utf8(buf).ok()?;
            let c = s.chars().next()?;
            Some((Some(Key::Char(c)), c.len_utf8()))
        }
        _ => Some((None, 1)), // other control bytes: consume + ignore
    }
}

/// Puts the terminal into raw mode and restores it on drop (and on explicit
/// `restore`). `panic = abort` means Drop will not run on a panic — the popup
/// pty being discarded by tmux is the backstop there.
pub struct RawMode {
    saved: String,
}

impl RawMode {
    pub fn enter() -> anyhow::Result<RawMode> {
        let saved = stty(&["-g"])?;
        let _ = stty(&["raw", "-echo"]);
        print!("\x1b[?25l"); // hide cursor
        std::io::stdout().flush().ok();
        Ok(RawMode { saved })
    }

    pub fn restore(&self) {
        let _ = stty(&[&self.saved]);
        print!("\x1b[?25h\x1b[2J\x1b[H"); // show cursor, clear screen
        std::io::stdout().flush().ok();
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        self.restore();
    }
}

fn stty(args: &[&str]) -> anyhow::Result<String> {
    use std::process::Stdio;
    // stty configures the terminal on *its stdin*. `Command::output()` would
    // default stdin to /dev/null, so it would silently fail to affect the real
    // terminal — inherit our controlling tty (the popup pty) instead.
    let out = Command::new("stty")
        .args(args)
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// (cols, rows), defaulting to 80x24 when unavailable.
pub fn term_size() -> (u16, u16) {
    if let Ok(s) = stty(&["size"]) {
        let mut it = s.split_whitespace();
        if let (Some(r), Some(c)) = (it.next(), it.next()) {
            if let (Ok(r), Ok(c)) = (r.parse::<u16>(), c.parse::<u16>()) {
                return (c, r);
            }
        }
    }
    (80, 24)
}

pub fn move_to(row: u16, col: u16) -> String {
    format!("\x1b[{row};{col}H")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_printable_and_controls() {
        assert!(matches!(parse_key(b"a"), Some((Some(Key::Char('a')), 1))));
        assert!(matches!(parse_key(b"\r"), Some((Some(Key::Enter), 1))));
        assert!(matches!(parse_key(b"\n"), Some((Some(Key::Enter), 1))));
        assert!(matches!(parse_key(&[0x7f]), Some((Some(Key::Backspace), 1))));
        assert!(matches!(parse_key(b"\t"), Some((Some(Key::Tab), 1))));
        assert!(matches!(parse_key(&[0x0e]), Some((Some(Key::Down), 1)))); // ^N
        assert!(matches!(parse_key(&[0x10]), Some((Some(Key::Up), 1)))); // ^P
        assert!(matches!(parse_key(&[0x12]), Some((Some(Key::Rename), 1))));
        assert!(matches!(parse_key(&[0x18]), Some((Some(Key::Delete), 1))));
        assert!(matches!(parse_key(&[0x03]), Some((Some(Key::Cancel), 1))));
        assert!(matches!(parse_key(&[0x1b]), Some((Some(Key::Cancel), 1))));
        assert!(matches!(parse_key(b"3"), Some((Some(Key::Digit(3)), 1))));
        assert!(matches!(parse_key(b"a"), Some((Some(Key::Char('a')), 1))));
    }

    #[test]
    fn parses_arrow_escape_sequences() {
        assert!(matches!(parse_key(b"\x1b[A"), Some((Some(Key::Up), 3))));
        assert!(matches!(parse_key(b"\x1b[B"), Some((Some(Key::Down), 3))));
        assert!(matches!(parse_key(b"\x1bOA"), Some((Some(Key::Up), 3)))); // SS3 form
    }

    #[test]
    fn incomplete_escape_needs_more() {
        assert_eq!(parse_key(b"\x1b["), None);
    }

    #[test]
    fn ctrl_letters_map_to_ctrl_variant() {
        assert_eq!(parse_key(&[0x16]), Some((Some(Key::Ctrl('v')), 1))); // ^V
        assert_eq!(parse_key(&[0x0f]), Some((Some(Key::Ctrl('o')), 1))); // ^O
        assert_eq!(parse_key(&[0x07]), Some((Some(Key::Ctrl('g')), 1))); // ^G (was bell)
        // specific control bindings still win (matched before the Ctrl range):
        assert_eq!(parse_key(&[0x12]), Some((Some(Key::Rename), 1))); // ^R
        assert_eq!(parse_key(&[0x10]), Some((Some(Key::Up), 1))); // ^P
    }
}

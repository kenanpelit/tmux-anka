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
            if buf[1] == b'[' {
                return parse_csi(buf);
            }
            if buf[1] == b'O' {
                // SS3: exactly one final byte follows `ESC O`.
                if buf.len() < 3 {
                    return None; // need the final byte
                }
                return match buf[2] {
                    b'A' => Some((Some(Key::Up), 3)),
                    b'B' => Some((Some(Key::Down), 3)),
                    _ => Some((None, 3)), // ignore other SS3 sequences
                };
            }
            Some((Some(Key::Cancel), 2)) // ESC + other → treat as cancel
        }
        b'1'..=b'9' => Some((Some(Key::Digit((b0 - b'0') as usize)), 1)),
        0x01..=0x1a => Some((Some(Key::Ctrl((b0 + 0x60) as char)), 1)),
        0x20..=0x7e => Some((Some(Key::Char(b0 as char)), 1)),
        0x80.. => parse_utf8(buf),
        _ => Some((None, 1)), // other control bytes: consume + ignore
    }
}

/// Parse a CSI sequence (`ESC [ params... final`) starting at `buf[0] == ESC`,
/// `buf[1] == '['`. Consumes parameter bytes (0x30-0x3F) and intermediate bytes
/// (0x20-0x2F) until a final byte (0x40-0x7E). A bare `ESC[A`/`ESC[B` (no
/// parameter/intermediate bytes) maps to Up/Down; every other complete
/// sequence — including modified arrows like `ESC[1;5A` (Ctrl-Up) — is
/// discarded whole so it can never leak trailing bytes for re-parsing (e.g.
/// as a query character or a row-activating Digit).
fn parse_csi(buf: &[u8]) -> Option<(Option<Key>, usize)> {
    let mut i = 2;
    loop {
        let b = *buf.get(i)?; // buffer ends mid-sequence: need more bytes
        if (0x20..=0x3f).contains(&b) {
            i += 1; // parameter / intermediate byte
            continue;
        }
        if (0x40..=0x7e).contains(&b) {
            let seq_len = i + 1;
            let bare = i == 2; // no parameter/intermediate bytes before the final byte
            return Some(match (b, bare) {
                (b'A', true) => (Some(Key::Up), seq_len),
                (b'B', true) => (Some(Key::Down), seq_len),
                _ => (None, seq_len),
            });
        }
        // Not a valid CSI byte — bail out rather than loop forever.
        return Some((None, i + 1));
    }
}

/// Decode the next key from a UTF-8 continuation/lead byte (`buf[0] >= 0x80`).
fn parse_utf8(buf: &[u8]) -> Option<(Option<Key>, usize)> {
    match std::str::from_utf8(buf) {
        Ok(s) => {
            let c = s.chars().next()?;
            Some((Some(Key::Char(c)), c.len_utf8()))
        }
        Err(e) if e.valid_up_to() > 0 => {
            // A valid character precedes the problem later in the buffer.
            let s = std::str::from_utf8(&buf[..e.valid_up_to()]).ok()?;
            let c = s.chars().next()?;
            Some((Some(Key::Char(c)), c.len_utf8()))
        }
        Err(e) => match e.error_len() {
            Some(n) => Some((None, n)), // invalid bytes lead: consume + ignore
            None if buf.len() >= 4 => Some((None, 1)), // guarantee progress
            None => None, // incomplete: await more bytes
        },
    }
}

/// Puts the terminal into raw mode and restores it on drop (and on explicit
/// `restore`). `panic = abort` means Drop will not run on a panic, so `enter`
/// also installs a panic hook that performs the same restore — panic hooks
/// run before the process aborts, so this covers the case Drop can't.
pub struct RawMode {
    saved: String,
}

/// Whether raw mode is currently live — the panic hook only restores the
/// terminal while this is true, so it's a no-op once `restore` has run.
static RAW_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The `stty -g` state to restore to, set each time raw mode is entered.
static SAVED_STTY: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static PANIC_HOOK_INSTALLED: std::sync::Once = std::sync::Once::new();

impl RawMode {
    pub fn enter() -> anyhow::Result<RawMode> {
        let saved = stty(&["-g"])?;
        install_panic_hook();
        if let Ok(mut slot) = SAVED_STTY.lock() {
            *slot = Some(saved.clone());
        }
        RAW_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = stty(&["raw", "-echo"]);
        print!("\x1b[?25l"); // hide cursor
        std::io::stdout().flush().ok();
        Ok(RawMode { saved })
    }

    pub fn restore(&self) {
        let _ = stty(&[&self.saved]);
        print!("\x1b[?25h\x1b[2J\x1b[H"); // show cursor, clear screen
        std::io::stdout().flush().ok();
        RAW_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Installs (once) a panic hook that restores the terminal before chaining to
/// whatever hook was previously registered. Guarded by `RAW_ACTIVE` so it's
/// inert outside a live raw-mode session (e.g. a panic in a non-interactive
/// subcommand doesn't print terminal-restore sequences to a pipe).
fn install_panic_hook() {
    PANIC_HOOK_INSTALLED.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if RAW_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
                if let Ok(guard) = SAVED_STTY.lock() {
                    if let Some(saved) = guard.as_deref() {
                        let _ = stty(&[saved]);
                    }
                }
                print!("\x1b[?25h\x1b[2J\x1b[H");
                std::io::stdout().flush().ok();
            }
            prev(info);
        }));
    });
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
    fn modified_arrow_with_params_is_ignored_whole_not_misread() {
        // Ctrl-Up (`\x1b[1;5A`) must not leak `;5A` for re-parsing as
        // Char(';') + Digit(5) (which would ACTIVATE row 5).
        assert_eq!(parse_key(b"\x1b[1;5A"), Some((None, 6)));
    }

    #[test]
    fn delete_key_csi_sequence_is_ignored_whole() {
        // `\x1b[3~` (Delete) must not leak `~` into the query.
        assert_eq!(parse_key(b"\x1b[3~"), Some((None, 4)));
    }

    #[test]
    fn bracketed_paste_guard_is_ignored_whole() {
        assert_eq!(parse_key(b"\x1b[200~"), Some((None, 6)));
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

    #[test]
    fn invalid_utf8_byte_is_consumed_and_ignored() {
        // A stray 0xff must not wedge the buffer forever.
        assert_eq!(parse_key(&[0xff]), Some((None, 1)));
    }

    #[test]
    fn valid_two_byte_utf8_still_decodes() {
        let bytes = "é".as_bytes(); // 0xC3 0xA9
        assert_eq!(parse_key(bytes), Some((Some(Key::Char('é')), 2)));
    }

    #[test]
    fn incomplete_utf8_lead_byte_awaits_more_bytes() {
        assert_eq!(parse_key(&[0xc3]), None); // awaiting continuation byte
    }
}

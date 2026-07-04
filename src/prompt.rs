//! Minimal one-line interactive prompt for `anka session … --prompt`,
//! reusing the switcher's raw-mode terminal layer. Designed for
//! `display-popup -E "anka session new --prompt"`: the popup tty hosts the
//! prompt, the typed name is used directly (no shell interpolation anywhere).
//! Off a tty (pipes, tests) it falls back to reading one line from stdin.
//! Empty input or ESC cancels silently.

use std::io::{self, IsTerminal, Read, Write};

use anyhow::Result;

use crate::switcher::term::{self, RawMode};
use crate::switcher::Key;

/// Outcome of applying one key to the prompt buffer.
#[derive(Debug, PartialEq)]
pub enum PromptStep {
    Pending,
    Done(String),
    Cancel,
}

/// Apply one decoded key to the input buffer (pure — unit-testable).
pub fn apply_key(buf: &mut String, key: Key) -> PromptStep {
    match key {
        Key::Char(c) => {
            buf.push(c);
            PromptStep::Pending
        }
        // In a prompt, digits are literal text (never a jump hotkey).
        Key::Digit(n) => {
            buf.push(char::from_digit(n as u32, 10).unwrap_or('0'));
            PromptStep::Pending
        }
        Key::Backspace => {
            buf.pop();
            PromptStep::Pending
        }
        Key::Enter => {
            let name = buf.trim();
            if name.is_empty() {
                PromptStep::Cancel
            } else {
                PromptStep::Done(name.to_string())
            }
        }
        Key::Cancel => PromptStep::Cancel,
        // Arrows / Tab / Ctrl-x: nothing to do on a single-line prompt.
        _ => PromptStep::Pending,
    }
}

/// Ask for a single line: raw-mode prompt on a tty, one stdin line otherwise.
/// `Ok(None)` = cancelled (empty input or ESC) — callers exit silently.
pub fn prompt_line(label: &str) -> Result<Option<String>> {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let name = line.trim();
        return Ok((!name.is_empty()).then(|| name.to_string()));
    }
    let raw = RawMode::enter()?;
    let mut buf = String::new();
    render(label, &buf);

    let mut stdin = io::stdin();
    let mut bytes: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 32];
    let result = 'outer: loop {
        let n = stdin.read(&mut chunk)?;
        if n == 0 {
            break None; // EOF
        }
        bytes.extend_from_slice(&chunk[..n]);
        while let Some((maybe, used)) = term::parse_key(&bytes) {
            bytes.drain(..used);
            let Some(key) = maybe else { continue };
            match apply_key(&mut buf, key) {
                PromptStep::Pending => render(label, &buf),
                PromptStep::Done(name) => break 'outer Some(name),
                PromptStep::Cancel => break 'outer None,
            }
        }
    };
    raw.restore();
    Ok(result)
}

fn render(label: &str, buf: &str) {
    print!("\x1b[2J\x1b[H\x1b[1;38;5;75m{label}\x1b[0m {buf}▌");
    io::stdout().flush().ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_str(buf: &mut String, s: &str) {
        for c in s.chars() {
            let key = match c {
                '1'..='9' => Key::Digit(c.to_digit(10).unwrap() as usize),
                _ => Key::Char(c),
            };
            assert_eq!(apply_key(buf, key), PromptStep::Pending);
        }
    }

    #[test]
    fn typing_then_enter_yields_trimmed_name() {
        let mut buf = String::new();
        type_str(&mut buf, " dev2 ");
        assert_eq!(apply_key(&mut buf, Key::Enter), PromptStep::Done("dev2".into()));
    }

    #[test]
    fn digits_are_literal_text() {
        let mut buf = String::new();
        type_str(&mut buf, "s8080");
        assert_eq!(buf, "s8080");
    }

    #[test]
    fn enter_on_empty_or_whitespace_cancels() {
        let mut buf = String::new();
        assert_eq!(apply_key(&mut buf, Key::Enter), PromptStep::Cancel);
        let mut buf = "   ".to_string();
        assert_eq!(apply_key(&mut buf, Key::Enter), PromptStep::Cancel);
    }

    #[test]
    fn esc_cancels_even_with_text() {
        let mut buf = "half-typed".to_string();
        assert_eq!(apply_key(&mut buf, Key::Cancel), PromptStep::Cancel);
    }

    #[test]
    fn backspace_edits_and_arrows_are_ignored() {
        let mut buf = String::new();
        type_str(&mut buf, "ab");
        assert_eq!(apply_key(&mut buf, Key::Backspace), PromptStep::Pending);
        assert_eq!(apply_key(&mut buf, Key::Up), PromptStep::Pending);
        assert_eq!(apply_key(&mut buf, Key::Tab), PromptStep::Pending);
        assert_eq!(apply_key(&mut buf, Key::Enter), PromptStep::Done("a".into()));
    }
}

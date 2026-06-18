//! Shared raw-mode fuzzy picker (used by `url` and `menu`). Bordered, single
//! column, fuzzy-filtered, 1-9 quick-jump — anka's own TUI, no `fzf`.

use std::io::{self, Read, Write};

use anyhow::Result;

use crate::switcher::term::{self, RawMode};
use crate::switcher::{fuzzy_score, Key};

const C_BORDER: &str = "\x1b[38;5;240m";
const C_TITLE: &str = "\x1b[1;38;5;75m";
const C_NUM: &str = "\x1b[38;5;220m";
const C_ACCENT: &str = "\x1b[38;5;75m";
const FG: &str = "\x1b[39m";
const R: &str = "\x1b[0m";

/// Outcome of an extended pick. `full` callers see Tab (cycle) and Ctrl accepts.
#[derive(Debug, PartialEq)]
pub enum Hit {
    Enter(usize),
    Ctrl(char, usize),
    Tab,
    Cancel,
}

fn run_picker(items: &[String], title: &str, full: bool) -> Result<Hit> {
    if items.is_empty() {
        return Ok(Hit::Cancel);
    }
    let raw = RawMode::enter()?;
    let mut query = String::new();
    let mut cursor = 0usize;
    let mut filtered: Vec<usize> = (0..items.len()).collect();

    render(items, &filtered, cursor, &query, title);
    let mut stdin = io::stdin();
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 32];
    loop {
        let n = stdin.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        while let Some((maybe, used)) = term::parse_key(&buf) {
            buf.drain(..used);
            let Some(key) = maybe else { continue };
            match key {
                Key::Cancel => {
                    raw.restore();
                    return Ok(Hit::Cancel);
                }
                Key::Enter => {
                    if let Some(&i) = filtered.get(cursor) {
                        raw.restore();
                        return Ok(Hit::Enter(i));
                    }
                }
                Key::Digit(d) if d >= 1 && d - 1 < filtered.len() => {
                    raw.restore();
                    return Ok(Hit::Enter(filtered[d - 1]));
                }
                Key::Tab if full => {
                    raw.restore();
                    return Ok(Hit::Tab);
                }
                Key::Ctrl(c) if full => {
                    if let Some(&i) = filtered.get(cursor) {
                        raw.restore();
                        return Ok(Hit::Ctrl(c, i));
                    }
                }
                Key::Up => cursor = cursor.saturating_sub(1),
                Key::Down if cursor + 1 < filtered.len() => cursor += 1,
                Key::Char(c) => {
                    query.push(c);
                    filtered = refilter(items, &query);
                    cursor = 0;
                }
                Key::Backspace => {
                    query.pop();
                    filtered = refilter(items, &query);
                    cursor = 0;
                }
                _ => {}
            }
            render(items, &filtered, cursor, &query, title);
        }
    }
    raw.restore();
    Ok(Hit::Cancel)
}

/// Interactive pick; returns the chosen index into `items`, or `None` on cancel.
pub fn pick(items: &[String], title: &str) -> Result<Option<usize>> {
    match run_picker(items, title, false)? {
        Hit::Enter(i) => Ok(Some(i)),
        _ => Ok(None),
    }
}

/// Like `pick`, but also surfaces Tab (cycle) and Ctrl-key accepts.
pub fn pick_ex(items: &[String], title: &str) -> Result<Hit> {
    run_picker(items, title, true)
}

/// Convenience: return the chosen item itself.
pub fn pick_str(items: &[String], title: &str) -> Result<Option<String>> {
    Ok(pick(items, title)?.map(|i| items[i].clone()))
}

fn refilter(items: &[String], query: &str) -> Vec<usize> {
    let mut scored: Vec<(usize, i32)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, u)| fuzzy_score(query, u).map(|s| (i, s)))
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.into_iter().map(|(i, _)| i).collect()
}

fn render(items: &[String], filtered: &[usize], cursor: usize, query: &str, title: &str) {
    let (cols, rows) = term::term_size();
    let cols = cols.max(20);
    let rows = rows.max(6);
    let bottom = rows - 1;
    let inner = (cols - 2) as usize;
    let list_h = (bottom - 3) as usize;

    let mut out = String::from("\x1b[2J");
    out.push_str(&term::move_to(1, 1));
    let head = format!("anka · {title} · {}", filtered.len());
    let tt: String = head.chars().take(inner.saturating_sub(3)).collect();
    let rem = inner.saturating_sub(3 + tt.chars().count());
    out.push_str(&format!("{C_BORDER}╭─ {C_TITLE}{tt}{C_BORDER} {}╮{R}", "─".repeat(rem)));

    let start = if cursor >= list_h && filtered.len() > list_h {
        (cursor + 1 - list_h).min(filtered.len() - list_h)
    } else {
        0
    };
    let body_w = inner.saturating_sub(4);
    for (row, idx) in (start..filtered.len().min(start + list_h)).enumerate() {
        let r = row as u16 + 2;
        out.push_str(&term::move_to(r, 1));
        out.push_str(&format!("{C_BORDER}│{R}"));
        out.push_str(&term::move_to(r, 2));
        let sel = idx == cursor;
        let bar = if sel {
            format!("{C_ACCENT}▌{R}")
        } else {
            " ".to_string()
        };
        out.push_str(&bar);
        let num = idx + 1;
        let badge = if num <= 9 { format!("{C_NUM}{num}{FG}") } else { " ".into() };
        let label: String = items[filtered[idx]].chars().take(body_w).collect();
        out.push_str(&term::move_to(r, 4));
        if sel {
            out.push_str("\x1b[1m");
        }
        out.push_str(&format!("{badge} {label}{R}"));
        out.push_str(&term::move_to(r, cols));
        out.push_str(&format!("{C_BORDER}│{R}"));
    }
    for row in (filtered.len().min(start + list_h) - start)..list_h {
        let r = row as u16 + 2;
        out.push_str(&term::move_to(r, 1));
        out.push_str(&format!("{C_BORDER}│{R}"));
        out.push_str(&term::move_to(r, cols));
        out.push_str(&format!("{C_BORDER}│{R}"));
    }
    out.push_str(&term::move_to(bottom - 1, 1));
    out.push_str(&format!("{C_BORDER}│{R}"));
    out.push_str(&term::move_to(bottom - 1, 3));
    let q: String = query.chars().take(body_w).collect();
    out.push_str(&format!("{C_ACCENT}›{R} {q}▌"));
    out.push_str(&term::move_to(bottom - 1, cols));
    out.push_str(&format!("{C_BORDER}│{R}"));
    out.push_str(&term::move_to(bottom, 1));
    out.push_str(&format!("{C_BORDER}╰{}╯{R}", "─".repeat(inner)));
    out.push_str(&term::move_to(rows, 1));
    out.push_str(&format!("{C_BORDER} ↑↓/^p^n · 1-9 jump · ⏎ select · esc{R}"));

    print!("{out}");
    io::stdout().flush().ok();
}

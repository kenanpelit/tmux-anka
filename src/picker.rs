//! Shared raw-mode fuzzy picker (used by `url` and `menu`). Bordered, single
//! column, fuzzy-filtered, 1-9 quick-jump — anka's own TUI, no `fzf`.

use std::io::{self, Read, Write};

use anyhow::Result;

use crate::switcher::term::{self, RawMode};
use crate::switcher::{fuzzy_positions, fuzzy_score, Key};

const C_BORDER: &str = "\x1b[38;5;240m";
const C_TITLE: &str = "\x1b[1;38;5;75m";
const C_NUM: &str = "\x1b[38;5;220m";
const C_ACCENT: &str = "\x1b[38;5;75m";
const C_MATCH: &str = "\x1b[1;38;5;214m";
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

/// Returns the outcome plus the final query string the user typed.
fn run_picker(items: &[String], title: &str, full: bool) -> Result<(Hit, String)> {
    if items.is_empty() {
        return Ok((Hit::Cancel, String::new()));
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
                    return Ok((Hit::Cancel, query));
                }
                Key::Enter => {
                    if let Some(&i) = filtered.get(cursor) {
                        raw.restore();
                        return Ok((Hit::Enter(i), query));
                    }
                }
                Key::Digit(d) if d >= 1 && d - 1 < filtered.len() => {
                    raw.restore();
                    return Ok((Hit::Enter(filtered[d - 1]), query));
                }
                Key::Tab if full => {
                    raw.restore();
                    return Ok((Hit::Tab, query));
                }
                Key::Ctrl(c) if full => {
                    if let Some(&i) = filtered.get(cursor) {
                        raw.restore();
                        return Ok((Hit::Ctrl(c, i), query));
                    }
                }
                // bottom-anchored: best is at the bottom, so Down moves toward it
                Key::Up if cursor + 1 < filtered.len() => cursor += 1,
                Key::Down => cursor = cursor.saturating_sub(1),
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
    Ok((Hit::Cancel, query))
}

/// Interactive pick; returns the chosen index into `items`, or `None` on cancel.
pub fn pick(items: &[String], title: &str) -> Result<Option<usize>> {
    match run_picker(items, title, false)?.0 {
        Hit::Enter(i) => Ok(Some(i)),
        _ => Ok(None),
    }
}

/// Like `pick`, but returns the chosen index together with the typed query.
pub fn pick_q(items: &[String], title: &str) -> Result<Option<(usize, String)>> {
    let (hit, query) = run_picker(items, title, false)?;
    Ok(match hit {
        Hit::Enter(i) => Some((i, query)),
        _ => None,
    })
}

/// Like `pick`, but also surfaces Tab (cycle) and Ctrl-key accepts.
pub fn pick_ex(items: &[String], title: &str) -> Result<Hit> {
    Ok(run_picker(items, title, true)?.0)
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

    let body_w = inner.saturating_sub(4);
    // fzf-style: results sit at the bottom (just above the query), best match
    // nearest it, growing upward. `list_bot` is the row above the query line.
    let list_bot = bottom - 2;
    let start = cursor.saturating_sub(list_h.saturating_sub(1));
    let visible = filtered.len().saturating_sub(start).min(list_h);
    // empty rows fill the TOP of the list area (above the results)
    for r in 2..(list_bot + 1 - visible as u16) {
        out.push_str(&term::move_to(r, 1));
        out.push_str(&format!("{C_BORDER}│{R}"));
        out.push_str(&term::move_to(r, cols));
        out.push_str(&format!("{C_BORDER}│{R}"));
    }
    // results: j=0 (best in window) on the bottom row, growing upward
    for j in 0..visible {
        let idx = start + j;
        let r = list_bot - j as u16;
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
        let item = &items[filtered[idx]];
        let positions = if query.is_empty() { None } else { fuzzy_positions(query, item) };
        let base = if sel { "\x1b[1m" } else { "" };
        out.push_str(&term::move_to(r, 4));
        out.push_str(&format!("{badge} {base}"));
        // highlight the chars matched by the live query (fzf-style)
        for (p, ch) in item.chars().take(body_w).enumerate() {
            if positions.as_ref().is_some_and(|v| v.contains(&p)) {
                out.push_str(&format!("{C_MATCH}{ch}{R}{base}"));
            } else {
                out.push(ch);
            }
        }
        out.push_str(R);
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

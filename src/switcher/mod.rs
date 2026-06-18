//! Native session switcher: an interactive picker over live + snapshot sessions
//! (and windows / zoxide dirs), split into a pure state machine and thin I/O.

mod state;
pub mod term;

pub use state::*;

use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Result};

use crate::config::Config;
use crate::model::Snapshot;
use crate::restore;
use crate::session;
use crate::store;
use crate::tmux;

/// `anka switch` (and `anka pick`): interactive when on a tty, numbered-menu
/// fallback otherwise (pipes, tests).
pub fn run() -> Result<()> {
    if !tmux::server_running() {
        bail!("no tmux server running");
    }
    let cfg = Config::load();
    let snap = load_last_snapshot();
    let items = mode_items(Mode::Sessions, snap.as_ref(), cfg.zoxide);
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        interactive(items, snap, &cfg)
    } else {
        fallback(items, snap.as_ref())
    }
}

fn load_last_snapshot() -> Option<Snapshot> {
    let name = store::last_name()?;
    restore::load_snapshot(&name).ok()
}

fn live_sessions() -> Vec<String> {
    tmux::run(&["list-sessions", "-F", "#{session_name}"])
        .map(|s| s.lines().map(String::from).collect())
        .unwrap_or_default()
}

fn mode_items(mode: Mode, snap: Option<&Snapshot>, zoxide: bool) -> Vec<Item> {
    match mode {
        Mode::Sessions => {
            let live = live_sessions();
            let snaps: Vec<String> = snap
                .map(|s| s.sessions.iter().map(|x| x.name.clone()).collect())
                .unwrap_or_default();
            build_session_items(&live, &snaps)
        }
        Mode::Windows => windows_items(),
        Mode::Zoxide => {
            if zoxide {
                zoxide_items()
            } else {
                vec![]
            }
        }
    }
}

fn windows_items() -> Vec<Item> {
    let fmt = "#{session_name}\t#{window_index}\t#{window_name}";
    let out = tmux::run(&["list-windows", "-a", "-F", fmt]).unwrap_or_default();
    out.lines()
        .filter_map(|l| {
            let mut f = l.splitn(3, '\t');
            let session = f.next()?.to_string();
            let index: u32 = f.next()?.parse().ok()?;
            let name = f.next().unwrap_or("").to_string();
            Some(Item::Window { session, index, name })
        })
        .collect()
}

fn zoxide_items() -> Vec<Item> {
    match Command::new("zoxide").args(["query", "-l"]).output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| Item::Zoxide(Path::new(l).to_path_buf()))
            .collect(),
        _ => vec![],
    }
}

// ── Effects ───────────────────────────────────────────────────────────────

fn activate(item: &Item, snap: Option<&Snapshot>) -> Result<()> {
    match item {
        Item::Live(n) => {
            tmux::run_ok(&["switch-client", "-t", n]);
        }
        Item::Snapshot(n) => {
            if let Some(s) = snap {
                restore::restore_one(s, n)?;
            }
        }
        Item::Window { session, index, .. } => {
            tmux::run_ok(&["switch-client", "-t", session]);
            tmux::run_ok(&["select-window", "-t", &format!("{session}:{index}")]);
        }
        Item::Zoxide(p) => {
            let base = p.file_name().and_then(|s| s.to_str()).unwrap_or("dir");
            session::new_in_dir(&sanitize(base), &p.display().to_string())?;
        }
    }
    Ok(())
}

fn do_kill(item: &Item) {
    if let Item::Live(n) = item {
        tmux::run_ok(&["kill-session", "-t", n]);
    }
}

fn do_rename(item: &Item, to: &str) {
    // Only live sessions exist in tmux to rename; snapshot rename is out of scope.
    if let Item::Live(n) = item {
        tmux::run_ok(&["rename-session", "-t", n, to]);
    }
}

/// tmux session names may not contain `.` or `:`.
fn sanitize(name: &str) -> String {
    name.replace(['.', ':'], "_")
}

// ── Non-tty fallback (numbered menu) ────────────────────────────────────────

fn fallback(items: Vec<Item>, snap: Option<&Snapshot>) -> Result<()> {
    if items.is_empty() {
        println!("(no sessions)");
        return Ok(());
    }
    println!("anka — switch session:\n");
    for (i, it) in items.iter().enumerate() {
        println!("  {:>2})  {}", i + 1, item_label(it));
    }
    print!("\nselect [1-{}], a name, or q: ", items.len());
    io::stdout().flush().ok();

    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let choice = line.trim();
    if choice.is_empty() || choice.eq_ignore_ascii_case("q") {
        println!("cancelled");
        return Ok(());
    }
    let chosen = choice
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=items.len()).contains(n))
        .map(|n| &items[n - 1])
        .or_else(|| best_match(&items, choice));
    match chosen {
        Some(it) => {
            activate(it, snap)?;
            println!("→ {}", item_label(it));
            Ok(())
        }
        None => bail!("no match for {choice:?}"),
    }
}

fn best_match<'a>(items: &'a [Item], query: &str) -> Option<&'a Item> {
    items
        .iter()
        .filter_map(|it| fuzzy_score(query, &item_key(it)).map(|s| (s, it)))
        .max_by_key(|(s, _)| *s)
        .map(|(_, it)| it)
}

// ── Interactive TUI ─────────────────────────────────────────────────────────

fn interactive(items: Vec<Item>, snap: Option<Snapshot>, cfg: &Config) -> Result<()> {
    let mut state = State::new(items, Mode::Sessions);
    let raw = term::RawMode::enter()?;
    let mut preview = String::new();
    let mut details = live_details();
    refresh_preview(&mut preview, state.selected(), snap.as_ref(), cfg);
    render(&state, &preview, cfg, &details, snap.as_ref());

    let mut stdin = io::stdin();
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 32];
    loop {
        let n = stdin.read(&mut chunk)?;
        if n == 0 {
            break; // EOF
        }
        buf.extend_from_slice(&chunk[..n]);
        // Drain every complete key in the buffer (an incomplete escape stops it).
        while let Some((maybe, consumed)) = term::parse_key(&buf) {
            buf.drain(..consumed);
            let Some(key) = maybe else { continue };

            let before = state.mode();
            let step = state.apply(key);
            if state.mode() != before {
                state.set_items(mode_items(state.mode(), snap.as_ref(), cfg.zoxide));
                refresh_preview(&mut preview, state.selected(), snap.as_ref(), cfg);
            }
            match step {
                Step::Redraw => render(&state, &preview, cfg, &details, snap.as_ref()),
                Step::PreviewChanged => {
                    refresh_preview(&mut preview, state.selected(), snap.as_ref(), cfg);
                    render(&state, &preview, cfg, &details, snap.as_ref());
                }
                Step::Stay(stay) => {
                    match stay {
                        Stay::Kill(it) => do_kill(&it),
                        Stay::Rename { item, to } => do_rename(&item, &to),
                    }
                    state.set_items(mode_items(state.mode(), snap.as_ref(), cfg.zoxide));
                    details = live_details();
                    refresh_preview(&mut preview, state.selected(), snap.as_ref(), cfg);
                    render(&state, &preview, cfg, &details, snap.as_ref());
                }
                Step::Exit(exit) => {
                    raw.restore();
                    return finish(exit, snap.as_ref());
                }
            }
        }
    }
    raw.restore();
    Ok(())
}

fn finish(exit: Exit, snap: Option<&Snapshot>) -> Result<()> {
    match exit {
        Exit::Activate(it) => activate(&it, snap),
        Exit::NewSession(name) => session::new_named(&name),
        Exit::Cancel => Ok(()),
    }
}

// ── Preview ─────────────────────────────────────────────────────────────────

fn refresh_preview(preview: &mut String, item: Option<&Item>, snap: Option<&Snapshot>, cfg: &Config) {
    if !cfg.switch_preview {
        return;
    }
    *preview = match item {
        Some(Item::Live(n)) => capture(n),
        Some(Item::Window { session, index, .. }) => capture(&format!("{session}:{index}")),
        Some(Item::Snapshot(n)) => snapshot_summary(snap, n),
        Some(Item::Zoxide(p)) => dir_listing(p),
        None => String::new(),
    };
}

/// Live pane contents *with* the program's own colours (`-e`), for an honest
/// preview like choose-tree's.
fn capture(target: &str) -> String {
    tmux::run(&["capture-pane", "-ep", "-t", target]).unwrap_or_default()
}

fn snapshot_summary(snap: Option<&Snapshot>, name: &str) -> String {
    let Some(sess) = snap.and_then(|s| s.sessions.iter().find(|x| x.name == name)) else {
        return String::new();
    };
    let mut out = format!("{C_DIM}offline snapshot{R}\n\n");
    for w in &sess.windows {
        out.push_str(&format!("{C_ACCENT}{}{R} {}  {C_DIM}{}p{R}\n", w.index, w.name, w.panes.len()));
        if let Some(p) = w.panes.first() {
            out.push_str(&format!("  {C_DIM}{}{R}\n", p.cwd));
        }
    }
    out
}

fn dir_listing(p: &Path) -> String {
    let mut out = format!("{C_ACCENT}{}{R}\n\n", p.display());
    match std::fs::read_dir(p) {
        Ok(rd) => {
            for e in rd.flatten().take(40) {
                out.push_str(&format!("  {}\n", e.file_name().to_string_lossy()));
            }
        }
        Err(_) => out.push_str(&format!("{C_DIM}(unreadable){R}\n")),
    }
    out
}

// ── Rendering ───────────────────────────────────────────────────────────────

type Details = std::collections::HashMap<String, (usize, usize)>; // session -> (windows, panes)

const C_BORDER: &str = "\x1b[38;5;240m";
const C_TITLE: &str = "\x1b[1;38;5;75m";
const C_NUM: &str = "\x1b[38;5;220m";
const C_LIVE: &str = "\x1b[38;5;114m";
const C_SNAP: &str = "\x1b[38;5;110m";
const C_DIM: &str = "\x1b[38;5;245m";
const C_ACCENT: &str = "\x1b[38;5;75m";
const BOLD: &str = "\x1b[1m";
const FG: &str = "\x1b[39m"; // reset foreground only (keeps bold/bg on selected rows)
const R: &str = "\x1b[0m";

/// Window/pane counts for live sessions, in two tmux calls.
fn live_details() -> Details {
    let mut win: Details = Details::new();
    if let Ok(o) = tmux::run(&["list-windows", "-a", "-F", "#{session_name}"]) {
        for l in o.lines() {
            win.entry(l.to_string()).or_default().0 += 1;
        }
    }
    if let Ok(o) = tmux::run(&["list-panes", "-a", "-F", "#{session_name}"]) {
        for l in o.lines() {
            win.entry(l.to_string()).or_default().1 += 1;
        }
    }
    win
}

/// The coloured `live · 3w 5p` / `snapshot · 2w 4p` tag (empty for window/zoxide).
fn detail_tag(item: &Item, details: &Details, snap: Option<&Snapshot>) -> String {
    match item {
        Item::Live(n) => {
            let (w, p) = details.get(n).copied().unwrap_or((0, 0));
            format!("{C_LIVE}live{FG} {C_DIM}· {w}w {p}p{FG}")
        }
        Item::Snapshot(n) => {
            let (w, p) = snap
                .and_then(|s| s.sessions.iter().find(|x| x.name == *n))
                .map(|s| (s.windows.len(), s.windows.iter().map(|w| w.panes.len()).sum()))
                .unwrap_or((0, 0));
            format!("{C_SNAP}snapshot{FG} {C_DIM}· {w}w {p}p{FG}")
        }
        _ => String::new(),
    }
}

fn render(state: &State, preview: &str, cfg: &Config, details: &Details, snap: Option<&Snapshot>) {
    let (cols, rows) = term::term_size();
    let cols = cols.max(20);
    let rows = rows.max(6);
    let preview_on = cfg.switch_preview && cols >= 80;
    let lw: u16 = if preview_on { (cols * 2 / 5).clamp(26, 52) } else { cols };
    let rw = cols - lw;
    let bottom = rows - 1; // boxes span rows 1..=bottom; footer on `rows`
    let inner_top = 2u16;
    let inner_bottom = bottom - 1;
    let inner_rows = (inner_bottom - inner_top + 1) as usize;
    let list_rows = inner_rows.saturating_sub(1); // last inner row = query/prompt

    let mut out = String::from("\x1b[2J");

    // Frames.
    let mode = match state.mode() {
        Mode::Sessions => "sessions",
        Mode::Windows => "windows",
        Mode::Zoxide => "zoxide",
    };
    draw_frame(&mut out, 1, lw, 1, bottom, &format!("anka · {mode}"));
    if preview_on {
        let title = state.selected().map(item_name).unwrap_or_default();
        draw_frame(&mut out, lw + 1, rw, 1, bottom, &format!("preview: {title}"));
    }

    // Left: numbered, coloured list.
    let vis = state.visible();
    let start = scroll_start(state.cursor(), vis.len(), list_rows);
    let body_w = (lw as usize).saturating_sub(6); // "│▌ N …" + right border + gutter
    for (i, idx) in (start..vis.len().min(start + list_rows)).enumerate() {
        let row = inner_top + i as u16;
        let sel = idx == state.cursor();
        let num = idx + 1;
        out.push_str(&term::move_to(row, 2));
        let bar = if sel {
            format!("{C_ACCENT}▌{R}")
        } else {
            " ".to_string()
        };
        out.push_str(&bar);
        let badge = if num <= 9 { format!("{C_NUM}{num}{FG}") } else { " ".into() };
        let tag = detail_tag(vis[idx], details, snap);
        let tagw = vis_width(&tag);
        let name = item_name(vis[idx]);
        let name_w = body_w.saturating_sub(if tagw > 0 { tagw + 2 } else { 0 });
        let name = truncate(&name, name_w);
        // pad name so the tag right-aligns-ish
        let pad = " ".repeat(name_w.saturating_sub(name.chars().count()));
        out.push_str(&term::move_to(row, 4));
        if sel {
            out.push_str(BOLD);
        }
        out.push_str(&badge);
        out.push(' ');
        out.push_str(&name);
        if tagw > 0 {
            out.push_str(&pad);
            out.push_str("  ");
            out.push_str(&tag);
        }
        out.push_str(R);
    }

    // Left bottom inner row: prompt or query.
    out.push_str(&term::move_to(inner_bottom, 3));
    if let Some((label, buf)) = state.prompt() {
        out.push_str(&truncate(&format!("{C_ACCENT}{label}{R} {buf}▌"), lw as usize));
    } else {
        out.push_str(&format!("{C_ACCENT}›{R} {}▌", truncate(state.query(), body_w)));
    }

    // Right: coloured preview.
    if preview_on {
        let pw = (rw as usize).saturating_sub(3);
        for (i, line) in preview.lines().take(inner_rows).enumerate() {
            out.push_str(&term::move_to(inner_top + i as u16, lw + 3));
            out.push_str(&ansi_truncate(line, pw));
        }
    }

    // Footer.
    out.push_str(&term::move_to(rows, 1));
    out.push_str(C_DIM);
    out.push_str(&truncate(
        " ↑↓/^p^n move · 1-9 jump · ⏎ go (type+⏎ new) · ^r rename · ^x kill · ⇥ mode · esc",
        cols as usize,
    ));
    out.push_str(R);

    print!("{out}");
    io::stdout().flush().ok();
}

/// Draw a rounded box from `(x, top)` of size `w × (bottom-top+1)`, titled.
fn draw_frame(out: &mut String, x: u16, w: u16, top: u16, bottom: u16, title: &str) {
    if w < 2 {
        return;
    }
    let inner = (w - 2) as usize;
    let tt = truncate(title, inner.saturating_sub(3));
    let rem = inner.saturating_sub(3 + tt.chars().count());
    out.push_str(&term::move_to(top, x));
    out.push_str(&format!(
        "{C_BORDER}╭─ {C_TITLE}{tt}{C_BORDER} {}╮{R}",
        "─".repeat(rem)
    ));
    for r in (top + 1)..bottom {
        out.push_str(&term::move_to(r, x));
        out.push_str(&format!("{C_BORDER}│{R}"));
        out.push_str(&term::move_to(r, x + w - 1));
        out.push_str(&format!("{C_BORDER}│{R}"));
    }
    out.push_str(&term::move_to(bottom, x));
    out.push_str(&format!("{C_BORDER}╰{}╯{R}", "─".repeat(inner)));
}

/// Visible width of a string, ignoring ANSI CSI escape sequences.
fn vis_width(s: &str) -> usize {
    let mut w = 0;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            w += 1;
        }
    }
    w
}

/// Truncate to `max` visible columns, preserving ANSI escapes, then reset.
fn ansi_truncate(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut w = 0;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            out.push(c);
            for c2 in chars.by_ref() {
                out.push(c2);
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            if w >= max {
                break;
            }
            out.push(c);
            w += 1;
        }
    }
    out.push_str(R);
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let keep = max.saturating_sub(1);
        s.chars().take(keep).collect::<String>() + "…"
    }
}

fn scroll_start(cursor: usize, len: usize, height: usize) -> usize {
    if height == 0 || len <= height || cursor < height {
        0
    } else {
        (cursor + 1 - height).min(len - height)
    }
}

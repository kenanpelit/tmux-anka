//! Native session switcher: an interactive picker over live + snapshot sessions
//! (and windows / zoxide dirs), split into a pure state machine and thin I/O.

mod state;
mod term;

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
    refresh_preview(&mut preview, state.selected(), snap.as_ref(), cfg);
    render(&state, &preview, cfg);

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
                Step::Redraw => render(&state, &preview, cfg),
                Step::PreviewChanged => {
                    refresh_preview(&mut preview, state.selected(), snap.as_ref(), cfg);
                    render(&state, &preview, cfg);
                }
                Step::Stay(stay) => {
                    match stay {
                        Stay::Kill(it) => do_kill(&it),
                        Stay::Rename { item, to } => do_rename(&item, &to),
                    }
                    state.set_items(mode_items(state.mode(), snap.as_ref(), cfg.zoxide));
                    refresh_preview(&mut preview, state.selected(), snap.as_ref(), cfg);
                    render(&state, &preview, cfg);
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

fn capture(target: &str) -> String {
    tmux::run(&["capture-pane", "-p", "-t", target]).unwrap_or_default()
}

fn snapshot_summary(snap: Option<&Snapshot>, name: &str) -> String {
    let Some(sess) = snap.and_then(|s| s.sessions.iter().find(|x| x.name == name)) else {
        return String::new();
    };
    let mut out = format!("snapshot session '{name}'\n");
    for w in &sess.windows {
        out.push_str(&format!("  win {} {} ({}p)\n", w.index, w.name, w.panes.len()));
        if let Some(p) = w.panes.first() {
            out.push_str(&format!("    {}\n", p.cwd));
        }
    }
    out
}

fn dir_listing(p: &Path) -> String {
    let mut out = format!("{}\n", p.display());
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten().take(20) {
            out.push_str(&format!("  {}\n", e.file_name().to_string_lossy()));
        }
    }
    out
}

// ── Rendering ───────────────────────────────────────────────────────────────

fn render(state: &State, preview: &str, cfg: &Config) {
    let (cols, rows) = term::term_size();
    let preview_on = cfg.switch_preview && cols >= 80;
    let left_w = if preview_on { (cols / 2) as usize } else { cols as usize };
    let list_h = rows.saturating_sub(3) as usize;

    let mut out = String::from(term::clear());
    let mode = match state.mode() {
        Mode::Sessions => "sessions",
        Mode::Windows => "windows",
        Mode::Zoxide => "zoxide",
    };
    out.push_str(&term::move_to(1, 1));
    out.push_str(&truncate(
        &format!("anka switch · {mode} · {} items", state.visible().len()),
        cols as usize,
    ));

    let vis = state.visible();
    let start = scroll_start(state.cursor(), vis.len(), list_h);
    for (row, idx) in (start..vis.len().min(start + list_h)).enumerate() {
        out.push_str(&term::move_to(row as u16 + 2, 1));
        // 1-based number (1-9 are jump hotkeys; rows past 9 show a space).
        let num = idx + 1;
        let tag = if num <= 9 { num.to_string() } else { " ".into() };
        let label = truncate(&item_label(vis[idx]), left_w.saturating_sub(3));
        if idx == state.cursor() {
            out.push_str(&format!("\x1b[7m{tag} {label}\x1b[0m"));
        } else {
            out.push_str(&format!("{tag} {label}"));
        }
    }

    if preview_on {
        let pw = cols as usize - left_w - 1;
        for (row, line) in preview.lines().take(list_h).enumerate() {
            out.push_str(&term::move_to(row as u16 + 2, left_w as u16 + 2));
            out.push_str(&truncate(line, pw));
        }
    }

    out.push_str(&term::move_to(rows.saturating_sub(1), 1));
    if let Some((label, buf)) = state.prompt() {
        out.push_str(&truncate(&format!("{label} {buf}_"), cols as usize));
    } else {
        out.push_str(&truncate(&format!("> {}", state.query()), cols as usize));
    }
    out.push_str(&term::move_to(rows, 1));
    out.push_str(&truncate(
        "↑↓/^p^n move · 1-9 jump · ⏎ go (type+⏎ new) · ^r rename · ^x kill · Tab mode · esc",
        cols as usize,
    ));

    print!("{out}");
    io::stdout().flush().ok();
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

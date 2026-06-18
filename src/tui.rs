//! Per-session lazy restore: a dependency-free interactive picker.
//!
//! Bound to `prefix + P` via `display-popup -E "anka pick"`. Lists the sessions
//! in the `last` snapshot, lets you choose one, and restores just that session
//! (saving memory vs. restoring everything). Deliberately free of any TUI crate
//! to keep the project's "one tiny static binary" promise — a numbered menu
//! reads cleanly inside the popup's tty.

use anyhow::{bail, Context, Result};
use std::io::{self, Write};

use crate::restore;
use crate::store;
use crate::tmux;

pub fn pick() -> Result<()> {
    if !tmux::server_running() {
        bail!("no tmux server running");
    }
    let name = store::last_name().context("no snapshot to pick from (no `last`)")?;
    let snap = restore::load_snapshot(&name)?;
    if snap.sessions.is_empty() {
        println!("snapshot '{name}' has no sessions");
        return Ok(());
    }

    let live = live_sessions();
    println!("anka — restore a session from snapshot '{name}':\n");
    for (i, s) in snap.sessions.iter().enumerate() {
        let panes: usize = s.windows.iter().map(|w| w.panes.len()).sum();
        let marker = if live.contains(&s.name) { "  (live)" } else { "" };
        println!(
            "  {:>2})  {:<24} {} win · {} panes{}",
            i + 1,
            s.name,
            s.windows.len(),
            panes,
            marker
        );
    }
    print!("\nselect [1-{}], or q to cancel: ", snap.sessions.len());
    io::stdout().flush().ok();

    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let choice = line.trim();
    if choice.is_empty() || choice.eq_ignore_ascii_case("q") {
        println!("cancelled");
        return Ok(());
    }
    let idx = choice
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=snap.sessions.len()).contains(n))
        .with_context(|| format!("invalid selection: {choice:?}"))?;

    let target = &snap.sessions[idx - 1].name;
    if restore::restore_one(&snap, target)? {
        println!("restored session '{target}'");
    } else {
        println!("session '{target}' is already live — switched to it");
        tmux::run_ok(&["switch-client", "-t", target]);
    }
    Ok(())
}

fn live_sessions() -> Vec<String> {
    tmux::run(&["list-sessions", "-F", "#{session_name}"])
        .map(|s| s.lines().map(String::from).collect())
        .unwrap_or_default()
}

//! Restore a snapshot back into tmux.
//!
//! Deterministic rebuild: recreate sessions (skipping live ones unless
//! `@anka-restore-overwrite`), their windows in index order, the panes as plain
//! shells, then apply the saved tmux layout for pixel-exact geometry. Programs
//! (process/nvim panes) are relaunched by typing into the pane's shell, so a
//! pane survives even if its command exits or fails (e.g. ssh).

use anyhow::{bail, Context, Result};
use std::fs;

use crate::config::Config;
use crate::model::{Pane, RestoreKind, Session, Snapshot, Window};
use crate::store;
use crate::tmux;

pub fn restore(name: Option<&str>) -> Result<()> {
    if !tmux::server_running() {
        bail!("no tmux server running");
    }
    let name = resolve_name(name)?;
    let snap = load(&name)?;
    let cfg = Config::load();

    let existing = existing_sessions();
    let mut restored = 0usize;
    for session in &snap.sessions {
        if existing.contains(&session.name) && !cfg.restore_overwrite {
            continue; // non-destructive: never clobber a live session
        }
        restore_session(session, &cfg)
            .with_context(|| format!("restoring session '{}'", session.name))?;
        restored += 1;
    }

    if let Some(active) = &snap.client.active_session {
        tmux::run_ok(&["switch-client", "-t", active]);
    }

    println!("restored snapshot '{name}' ({restored} sessions)");
    Ok(())
}

/// Restore-on-start guard (called by `anka.tmux` on plugin load). Restores the
/// `last` snapshot at most once per tmux server, gated by the `@anka_restored`
/// option which records the server pid that was already restored.
pub fn autostart() -> Result<()> {
    let cfg = Config::load();
    if !cfg.restore_on_start {
        return Ok(());
    }
    let pid = tmux::run(&["display-message", "-p", "#{pid}"]).unwrap_or_default();
    if !pid.is_empty() && tmux::global_option("@anka_restored") == pid {
        return Ok(()); // already restored for this server
    }
    if store::last_name().is_some() {
        restore(None)?;
    }
    if !pid.is_empty() {
        tmux::set_global_option("@anka_restored", &pid);
    }
    Ok(())
}

fn restore_session(session: &Session, cfg: &Config) -> Result<()> {
    let mut windows: Vec<&Window> = session.windows.iter().collect();
    windows.sort_by_key(|w| w.index);

    let mut active_anchor: Option<String> = None;

    for (wi, w) in windows.iter().enumerate() {
        let mut panes: Vec<&Pane> = w.panes.iter().collect();
        panes.sort_by_key(|p| p.index);
        let first = panes.first().copied();
        let first_cwd = first.map(|p| p.cwd.clone()).unwrap_or_else(|| ".".to_string());

        // Create the window's first pane.
        let mut v: Vec<String> = Vec::new();
        if wi == 0 {
            v.extend(
                ["new-session", "-d", "-P", "-F", "#{pane_id}", "-s", session.name.as_str()]
                    .map(String::from),
            );
        } else {
            v.extend(["new-window", "-d", "-P", "-F", "#{pane_id}", "-t"].map(String::from));
            v.push(format!("{}:", session.name));
        }
        if !w.name.is_empty() {
            v.push("-n".into());
            v.push(w.name.clone());
        }
        v.push("-c".into());
        v.push(first_cwd);
        if wi == 0 {
            v.extend(["-x", "200", "-y", "50"].map(String::from));
        }
        let first_pane_id = run_str(&v)?;
        let mut pane_ids = vec![first_pane_id.clone()];

        // Remaining panes: split off the first pane (plain shells), fix geometry below.
        for p in panes.iter().skip(1) {
            let mut sv: Vec<String> =
                ["split-window", "-d", "-P", "-F", "#{pane_id}", "-t", first_pane_id.as_str(), "-c"]
                    .map(String::from)
                    .to_vec();
            sv.push(p.cwd.clone());
            if let Ok(pid) = run_str(&sv) {
                pane_ids.push(pid);
            }
        }

        if !w.layout.is_empty() {
            tmux::run_ok(&["select-layout", "-t", &first_pane_id, &w.layout]);
        }

        // Relaunch programs by typing into each pane's shell, so the pane
        // survives if the command exits or fails (ssh, editors, REPLs, …).
        for (p, pid) in panes.iter().zip(pane_ids.iter()) {
            if let Some(cmd) = launch_cmd(Some(p), cfg) {
                tmux::run_ok(&["send-keys", "-t", pid, "-l", &cmd]);
                tmux::run_ok(&["send-keys", "-t", pid, "Enter"]);
            }
        }

        // Active pane within the window (by position among created panes).
        if let Some(pos) = panes.iter().position(|p| p.active) {
            if let Some(pid) = pane_ids.get(pos) {
                tmux::run_ok(&["select-pane", "-t", pid]);
            }
        }

        if w.active {
            active_anchor = Some(first_pane_id);
        }
    }

    if let Some(anchor) = active_anchor {
        tmux::run_ok(&["select-window", "-t", &anchor]);
    }
    Ok(())
}

/// The command to launch a pane with, or `None` for a plain shell.
fn launch_cmd(pane: Option<&Pane>, _cfg: &Config) -> Option<String> {
    let p = pane?;
    match p.restore.kind {
        RestoreKind::Shell => None,
        RestoreKind::Process => p.restore.command.clone(),
        RestoreKind::Nvim => Some("nvim".to_string()),
    }
}

fn resolve_name(name: Option<&str>) -> Result<String> {
    match name {
        Some(n) => Ok(n.to_string()),
        None => store::last_name().context("no snapshot to restore (no `last`)"),
    }
}

fn load(name: &str) -> Result<Snapshot> {
    let path = store::snapshot_json(name);
    let data =
        fs::read(&path).with_context(|| format!("reading snapshot {}", path.display()))?;
    serde_json::from_slice(&data).with_context(|| format!("parsing snapshot {}", path.display()))
}

fn existing_sessions() -> Vec<String> {
    tmux::run(&["list-sessions", "-F", "#{session_name}"])
        .map(|s| s.lines().map(String::from).collect())
        .unwrap_or_default()
}

fn run_str(args: &[String]) -> Result<String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    tmux::run(&a)
}

//! Sessionist-style quick session actions.

use anyhow::{bail, Result};

use crate::cli::SessionCmd;
use crate::tmux;

pub fn run(action: SessionCmd) -> Result<()> {
    if !tmux::server_running() {
        bail!("no tmux server running");
    }
    match action {
        SessionCmd::New { name } => new_named(&name),
        SessionCmd::Kill => kill(),
        SessionCmd::Promote { name } => promote(&name),
        SessionCmd::Switch { name } => switch(&name),
        SessionCmd::Last => last(),
        SessionCmd::Rename { name } => rename(&name),
    }
}

/// The pane anka was invoked from: `$TMUX_PANE` (set by tmux for run-shell and
/// popups), else the server's current pane.
fn current_pane() -> Option<String> {
    if let Ok(p) = std::env::var("TMUX_PANE") {
        if !p.is_empty() {
            return Some(p);
        }
    }
    tmux::run(&["display-message", "-p", "#{pane_id}"])
        .ok()
        .filter(|s| !s.is_empty())
}

fn pane_var(pane: &str, fmt: &str) -> String {
    tmux::run(&["display-message", "-p", "-t", pane, fmt]).unwrap_or_default()
}

/// The session anka was invoked from, or the first session as a last resort.
fn current_session() -> Result<String> {
    if let Some(p) = current_pane() {
        let s = pane_var(&p, "#{session_name}");
        if !s.is_empty() {
            return Ok(s);
        }
    }
    sessions()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no sessions"))
}

fn sessions() -> Vec<String> {
    tmux::run(&["list-sessions", "-F", "#{session_name}"])
        .map(|s| s.lines().map(String::from).collect())
        .unwrap_or_default()
}

fn current_cwd() -> String {
    current_pane()
        .map(|p| pane_var(&p, "#{pane_current_path}"))
        .unwrap_or_default()
}

/// Create (or switch to) a session named `name`, rooted at the current cwd.
pub fn new_named(name: &str) -> Result<()> {
    if sessions().iter().any(|s| s == name) {
        tmux::run_ok(&["switch-client", "-t", name]);
        println!("switched to existing session '{name}'");
        return Ok(());
    }
    new_in_dir(name, &current_cwd())
}

/// Create a session named `name` rooted at `dir` (empty = tmux default), switch.
pub fn new_in_dir(name: &str, dir: &str) -> Result<()> {
    let mut args = vec!["new-session", "-d", "-s", name];
    if !dir.is_empty() {
        args.push("-c");
        args.push(dir);
    }
    tmux::run(&args)?;
    tmux::run_ok(&["switch-client", "-t", name]);
    println!("created session '{name}'");
    Ok(())
}

fn kill() -> Result<()> {
    let cur = current_session()?;
    let others: Vec<String> = sessions().into_iter().filter(|s| *s != cur).collect();
    let Some(target) = others.first() else {
        bail!("refusing to kill the last session '{cur}' (would stop the server)");
    };
    tmux::run_ok(&["switch-client", "-t", target]);
    tmux::run(&["kill-session", "-t", &cur])?;
    println!("killed session '{cur}', switched to '{target}'");
    Ok(())
}

fn promote(name: &str) -> Result<()> {
    if sessions().iter().any(|s| s == name) {
        bail!("session '{name}' already exists");
    }
    let pane = current_pane().ok_or_else(|| anyhow::anyhow!("no current pane to promote"))?;
    let cwd = pane_var(&pane, "#{pane_current_path}");
    // Create a placeholder session, move our pane in beside it, drop the placeholder.
    let mut args = vec!["new-session", "-d", "-s", name];
    if !cwd.is_empty() {
        args.push("-c");
        args.push(&cwd);
    }
    tmux::run(&args)?;
    let placeholder = pane_var(name, "#{pane_id}");
    tmux::run(&["move-pane", "-s", &pane, "-t", name])?;
    if !placeholder.is_empty() {
        tmux::run_ok(&["kill-pane", "-t", &placeholder]);
    }
    tmux::run_ok(&["select-layout", "-t", name, "tiled"]);
    tmux::run_ok(&["switch-client", "-t", name]);
    println!("promoted pane into new session '{name}'");
    Ok(())
}

fn switch(name: &str) -> Result<()> {
    if !sessions().iter().any(|s| s == name) {
        bail!("session '{name}' not found");
    }
    tmux::run(&["switch-client", "-t", name])?;
    Ok(())
}

fn last() -> Result<()> {
    tmux::run_ok(&["switch-client", "-l"]);
    Ok(())
}

fn rename(name: &str) -> Result<()> {
    let cur = current_session()?;
    tmux::run(&["rename-session", "-t", &cur, name])?;
    println!("renamed session '{cur}' to '{name}'");
    Ok(())
}

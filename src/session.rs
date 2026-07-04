//! Sessionist-style quick session actions.
//!
//! Success/failure is reported through `notify` (display-message inside
//! run-shell contexts, stdout on a terminal), and every tmux `-t` that names a
//! stored/derived session uses the exact-match `=` prefix so a session called
//! `dev` can never resolve onto `dev2`.

use anyhow::{bail, Result};

use crate::cli::SessionCmd;
use crate::notify;
use crate::prompt;
use crate::tmux;

pub fn run(action: SessionCmd) -> Result<()> {
    if !tmux::server_running() {
        bail!("no tmux server running");
    }
    match action {
        SessionCmd::New { name, prompt } => {
            match resolve_name(name, prompt, "New session:")? {
                Some(n) => new_named(&n),
                None => Ok(()), // cancelled: exit silently
            }
        }
        SessionCmd::Kill => kill(),
        SessionCmd::Promote { name, prompt } => {
            match resolve_name(name, prompt, "Promote pane to session:")? {
                Some(n) => promote(&n),
                None => Ok(()),
            }
        }
        SessionCmd::Switch { name, prompt } => {
            match resolve_name(name, prompt, "Switch to session:")? {
                Some(n) => switch(&n),
                None => Ok(()),
            }
        }
        SessionCmd::Last => last(),
        SessionCmd::Rename { name } => rename(&name),
    }
}

/// The action's target name: the CLI argument when given, else (with
/// `--prompt`) a one-line interactive prompt in the popup tty (stdin line off
/// a tty). `Ok(None)` = cancelled — the caller exits silently. The typed name
/// is passed as a plain argv element everywhere: no shell interpolation.
fn resolve_name(name: Option<String>, use_prompt: bool, label: &str) -> Result<Option<String>> {
    if let Some(n) = name {
        return Ok(Some(n));
    }
    if use_prompt {
        return prompt::prompt_line(label);
    }
    bail!("a session name is required (or --prompt)");
}

/// Exact-match tmux target for a session name (`=` pins exact, otherwise
/// tmux's prefix matching could land on the wrong session).
fn exact(name: &str) -> String {
    format!("={name}")
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

fn session_exists(name: &str) -> bool {
    sessions().iter().any(|s| s == name)
}

fn current_cwd() -> String {
    current_pane()
        .map(|p| pane_var(&p, "#{pane_current_path}"))
        .unwrap_or_default()
}

/// Create (or switch to) a session named `name`, rooted at the current cwd.
pub fn new_named(name: &str) -> Result<()> {
    new_in_dir(name, &current_cwd())
}

/// Create a session named `name` rooted at `dir` (empty = tmux default), then
/// switch to it. If the session already exists (a zoxide dir whose basename
/// collides with a live session, a re-typed name, …) just switch — same
/// behavior as `new_named`, instead of dying on tmux's "duplicate session".
pub fn new_in_dir(name: &str, dir: &str) -> Result<()> {
    if session_exists(name) {
        tmux::run_ok(&["switch-client", "-t", &exact(name)]);
        notify::success(&format!("switched to existing session '{name}'"));
        return Ok(());
    }
    let mut args = vec!["new-session", "-d", "-s", name];
    if !dir.is_empty() {
        args.push("-c");
        args.push(dir);
    }
    tmux::run(&args)?;
    tmux::run_ok(&["switch-client", "-t", &exact(name)]);
    notify::success(&format!("created session '{name}'"));
    Ok(())
}

fn kill() -> Result<()> {
    let cur = current_session()?;
    let others: Vec<String> = sessions().into_iter().filter(|s| *s != cur).collect();
    let Some(target) = others.first() else {
        bail!("refusing to kill the last session '{cur}' (would stop the server)");
    };
    tmux::run_ok(&["switch-client", "-t", &exact(target)]);
    tmux::run(&["kill-session", "-t", &exact(&cur)])?;
    notify::success(&format!("killed session '{cur}', switched to '{target}'"));
    Ok(())
}

fn promote(name: &str) -> Result<()> {
    if session_exists(name) {
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
    // NOTE: move-pane/select-layout/display-message take *pane* targets, and
    // tmux's `=` exact-match prefix is only valid for session/window targets
    // ("can't find pane: =x") — so these use the plain name. The freshly
    // created session resolves exactly anyway (tmux prefers exact matches).
    let placeholder = pane_var(name, "#{pane_id}");
    tmux::run(&["move-pane", "-s", &pane, "-t", name])?;
    if !placeholder.is_empty() {
        tmux::run_ok(&["kill-pane", "-t", &placeholder]);
    }
    tmux::run_ok(&["select-layout", "-t", name, "tiled"]);
    tmux::run_ok(&["switch-client", "-t", &exact(name)]);
    notify::success(&format!("promoted pane into new session '{name}'"));
    Ok(())
}

fn switch(name: &str) -> Result<()> {
    if !session_exists(name) {
        bail!("session '{name}' not found");
    }
    tmux::run(&["switch-client", "-t", &exact(name)])?;
    Ok(())
}

fn last() -> Result<()> {
    tmux::run_ok(&["switch-client", "-l"]);
    Ok(())
}

fn rename(name: &str) -> Result<()> {
    let cur = current_session()?;
    tmux::run(&["rename-session", "-t", &exact(&cur), name])?;
    notify::success(&format!("renamed session '{cur}' to '{name}'"));
    Ok(())
}

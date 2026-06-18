//! Thin wrapper around the `tmux` CLI.

use anyhow::{bail, Context, Result};
use std::process::Command;

/// Run a tmux command and return its stdout (trailing newline trimmed).
pub fn run(args: &[&str]) -> Result<String> {
    let out = Command::new("tmux")
        .args(args)
        .output()
        .context("failed to spawn tmux")?;
    if !out.status.success() {
        bail!(
            "tmux {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .trim_end_matches('\n')
        .to_string())
}

/// Run a tmux command, ignoring success/failure (best-effort).
pub fn run_ok(args: &[&str]) {
    let _ = Command::new("tmux").args(args).output();
}

/// Whether a tmux server is reachable.
pub fn server_running() -> bool {
    Command::new("tmux")
        .arg("list-sessions")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Read a global option value (e.g. `@anka-foo`). Empty string when unset.
pub fn global_option(name: &str) -> String {
    run(&["show-options", "-gqv", name]).unwrap_or_default()
}

/// Set a global option (best-effort).
pub fn set_global_option(name: &str, value: &str) {
    run_ok(&["set-option", "-g", name, value]);
}

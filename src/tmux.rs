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

/// Normalize the US (0x1f) field separator in `-F` output across tmux
/// versions: tmux <= 3.4 octal-escapes control bytes when printing list/`-F`
/// output, so our separator arrives as the literal four characters `\037`;
/// tmux >= 3.5 prints the raw byte. Callers split on the raw byte, so map
/// the escaped form back. (Cost: free text that literally contains `\037`
/// gains a spurious split — the free-text field is deliberately last in
/// every format we parse, so structured fields stay correct.)
pub fn unescape_us(s: &str) -> String {
    s.replace("\\037", "\u{1f}")
}

/// Read a global option value (e.g. `@anka-foo`). Empty string when unset.
pub fn global_option(name: &str) -> String {
    run(&["show-options", "-gqv", name]).unwrap_or_default()
}

/// Set a global option (best-effort).
pub fn set_global_option(name: &str, value: &str) {
    run_ok(&["set-option", "-g", name, value]);
}

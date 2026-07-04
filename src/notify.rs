//! User-visible messages that survive popup teardown.
//!
//! `eprintln!`/`println!` in a popup or `run-shell` context either vanishes
//! with the popup or lands in tmux's annoying q-to-dismiss output view. Inside
//! tmux, errors and successes are therefore surfaced with `display-message`
//! (the status line outlives the popup); outside tmux they go to the usual
//! stdio streams.

use std::io::IsTerminal;

use crate::tmux;

/// Whether we're running under tmux (popup, run-shell, hook, …).
fn in_tmux() -> bool {
    std::env::var_os("TMUX").is_some_and(|v| !v.is_empty())
}

/// Where a success message belongs: stdout for direct CLI use (a real
/// terminal, or no tmux at all), `display-message` otherwise — inside tmux a
/// non-tty stdout means run-shell's dismiss-with-q view.
pub fn wants_stdout(tmux_set: bool, stdout_is_tty: bool) -> bool {
    !tmux_set || stdout_is_tty
}

/// Report a successful action (`created session 'x'`, `saved snapshot …`).
pub fn success(msg: &str) {
    if wants_stdout(in_tmux(), std::io::stdout().is_terminal()) {
        println!("{msg}");
    } else {
        tmux::run_ok(&["display-message", &format!("anka: {msg}")]);
    }
}

/// Report an error. Always written to stderr (scripts/tests see it); inside
/// tmux ALSO shown via `display-message`, because a popup's stderr vanishes
/// with the popup.
pub fn error(msg: &str) {
    eprintln!("anka: {msg}");
    if in_tmux() {
        tmux::run_ok(&["display-message", &format!("anka: {msg}")]);
    }
}

/// Informational notice (`no URLs in this pane`, …): `display-message` inside
/// tmux, stdout otherwise.
pub fn info(msg: &str) {
    if in_tmux() {
        tmux::run_ok(&["display-message", &format!("anka: {msg}")]);
    } else {
        println!("anka: {msg}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdout_only_for_tty_or_outside_tmux() {
        // Outside tmux: always stdout (plain CLI use, piped or not).
        assert!(wants_stdout(false, true));
        assert!(wants_stdout(false, false));
        // Inside tmux: stdout only when it's a real terminal (a popup);
        // a pipe means run-shell's q-to-dismiss view → display-message.
        assert!(wants_stdout(true, true));
        assert!(!wants_stdout(true, false));
    }
}

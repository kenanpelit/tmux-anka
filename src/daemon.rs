//! Auto-save: event-driven hooks (working now) and the interval daemon (later).

use anyhow::Result;

use crate::capture;

/// Interval auto-save daemon. The periodic loop lands in v0.4.0; for now this
/// exits cleanly so the backgrounded process from `anka.tmux` is a no-op.
pub fn run() -> Result<()> {
    Ok(())
}

/// Event-driven save, invoked by tmux hooks (`session-closed`,
/// `client-detached`, …). Saves the default snapshot.
pub fn hook(_event: &str) -> Result<()> {
    capture::save(None)
}

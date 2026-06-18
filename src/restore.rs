//! Restore a snapshot back into tmux.
//!
//! The full deterministic rebuild lands in v0.2.0. The function is stubbed so
//! the CLI and keybindings are wired end to end.

use anyhow::{bail, Result};

pub fn restore(_name: Option<&str>) -> Result<()> {
    bail!("restore is not implemented yet (planned for v0.2.0)");
}

/// Restore-on-start guard, invoked by `anka.tmux` on plugin load. No-op until
/// restore exists; will become a once-per-server guarded `restore(last)`.
pub fn autostart() -> Result<()> {
    Ok(())
}

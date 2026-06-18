//! The `#{@anka_status}` status-bar widget text.

use anyhow::Result;

use crate::tmux;

pub fn print() -> Result<()> {
    // The binary writes @anka_status on every save; just echo it back.
    print!("{}", tmux::global_option("@anka_status"));
    Ok(())
}

//! Freeze a snapshot into a re-runnable blueprint (declarative spec + optional
//! shell-script export), and re-launch it with `anka up`.
//!
//! Lands in v0.6.0; stubbed so the CLI surface is stable.

use anyhow::{bail, Result};

pub fn freeze(_name: Option<&str>, _script: bool) -> Result<()> {
    bail!("freeze is not implemented yet (planned for v0.6.0)");
}

pub fn up(_name: &str) -> Result<()> {
    bail!("up is not implemented yet (planned for v0.6.0)");
}

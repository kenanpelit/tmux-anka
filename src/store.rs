//! Snapshot storage: directory layout, the `last` symlink, list/rm.

use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs as unixfs;
use std::path::{Path, PathBuf};

use crate::tmux;

pub const DEFAULT_SNAPSHOT: &str = "default";

/// `@anka-dir`, or `${XDG_DATA_HOME:-~/.local/share}/tmux/anka`.
pub fn base_dir() -> PathBuf {
    let configured = tmux::global_option("@anka-dir");
    if !configured.is_empty() {
        return expand(&configured);
    }
    let data = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    data.join("tmux").join("anka")
}

fn expand(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(p)
}

pub fn snapshots_dir() -> PathBuf {
    base_dir().join("snapshots")
}

pub fn snapshot_dir(name: &str) -> PathBuf {
    snapshots_dir().join(name)
}

pub fn snapshot_json(name: &str) -> PathBuf {
    snapshot_dir(name).join("snapshot.json")
}

pub fn last_link() -> PathBuf {
    snapshots_dir().join("last")
}

/// The name the `last` symlink points at, if any.
pub fn last_name() -> Option<String> {
    let target = fs::read_link(last_link()).ok()?;
    target
        .file_name()
        .and_then(|s| s.to_str())
        .map(String::from)
}

/// Point `last` at `name` (atomic replace).
pub fn set_last(name: &str) -> Result<()> {
    let link = last_link();
    let tmp = snapshots_dir().join(".last.tmp");
    let _ = fs::remove_file(&tmp);
    unixfs::symlink(name, &tmp).context("creating last symlink")?;
    fs::rename(&tmp, &link).context("activating last symlink")?;
    Ok(())
}

pub fn list_names() -> Vec<String> {
    let mut names: Vec<String> = match fs::read_dir(snapshots_dir()) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
}

pub fn list_cmd() -> Result<()> {
    let last = last_name();
    let names = list_names();
    if names.is_empty() {
        println!("(no snapshots)");
        return Ok(());
    }
    for name in names {
        let marker = if Some(&name) == last.as_ref() {
            " (last)"
        } else {
            ""
        };
        println!("{name}{marker}");
    }
    Ok(())
}

pub fn rm_cmd(name: &str) -> Result<()> {
    let dir = snapshot_dir(name);
    if !dir.is_dir() {
        bail!("snapshot '{name}' not found");
    }
    fs::remove_dir_all(&dir).with_context(|| format!("removing snapshot '{name}'"))?;
    if last_name().as_deref() == Some(name) {
        let _ = fs::remove_file(last_link());
    }
    println!("removed snapshot '{name}'");
    Ok(())
}

/// Write bytes atomically (tmp + rename in the same directory).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("activating {}", path.display()))?;
    Ok(())
}

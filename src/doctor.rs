//! `anka doctor` — environment / install sanity checks. Prints one PASS/FAIL
//! line per check and fails (exit 1) when anything FAILs.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Result};

use crate::daemon;
use crate::model;
use crate::store;

pub struct Check {
    pub ok: bool,
    pub label: String,
}

fn check(ok: bool, label: impl Into<String>) -> Check {
    Check { ok, label: label.into() }
}

pub fn run() -> Result<()> {
    let checks = run_checks();
    let failed = checks.iter().filter(|c| !c.ok).count();
    for c in &checks {
        println!("{} {}", if c.ok { "PASS" } else { "FAIL" }, c.label);
    }
    if failed > 0 {
        bail!("doctor: {failed} check(s) failed");
    }
    Ok(())
}

fn run_checks() -> Vec<Check> {
    vec![
        check_tmux(),
        check_version(),
        check_store_dir(),
        check_daemon(),
        check_default_snapshot(),
    ]
}

/// tmux on PATH, reporting its version.
fn check_tmux() -> Check {
    match Command::new("tmux").arg("-V").output() {
        Ok(o) if o.status.success() => {
            let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
            check(true, format!("tmux on PATH ({v})"))
        }
        _ => check(false, "tmux not found on PATH"),
    }
}

/// Binary version vs the plugin's Cargo.toml version (best effort: the binary
/// lives in `<plugin>/bin/`, so the plugin dir is `../` from the exe's dir).
/// A mismatch means the plugin was updated but the binary wasn't reinstalled
/// (or vice versa).
fn check_version() -> Check {
    let bin = env!("CARGO_PKG_VERSION");
    match plugin_version() {
        Some((path, ver)) if ver == bin => {
            check(true, format!("binary {bin} matches plugin {} ", path.display()))
        }
        Some((path, ver)) => check(
            false,
            format!("binary {bin} != plugin {ver} ({}) — reinstall the binary or reload the plugin", path.display()),
        ),
        None => check(true, format!("binary {bin} (plugin Cargo.toml not found — version check skipped)")),
    }
}

fn plugin_version() -> Option<(PathBuf, String)> {
    let exe = std::env::current_exe().ok()?;
    let plugin = exe.parent()?.parent()?; // <plugin>/bin/anka → <plugin>
    let toml_path = plugin.join("Cargo.toml");
    let text = std::fs::read_to_string(&toml_path).ok()?;
    Some((toml_path, cargo_version(&text)?))
}

/// Pull `version = "x.y.z"` out of a Cargo.toml (first match — `[package]`
/// comes first in ours).
pub fn cargo_version(toml: &str) -> Option<String> {
    for line in toml.lines() {
        let Some(rest) = line.trim().strip_prefix("version") else {
            continue;
        };
        if let Some(v) = rest.trim_start().strip_prefix('=') {
            return Some(v.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// Store dir exists (created if missing, exactly like a save would) and is
/// writable.
fn check_store_dir() -> Check {
    let dir = store::base_dir();
    match store_writable(&dir) {
        Ok(()) => check(true, format!("store dir writable ({})", dir.display())),
        Err(e) => check(false, format!("store dir {} not writable: {e}", dir.display())),
    }
}

fn store_writable(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(format!(".doctor-probe-{}", std::process::id()));
    std::fs::write(&probe, b"ok")?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// Daemon lock state: absent = fine (interval saves off / not started);
/// present must name a live process whose comm is `anka`.
fn check_daemon() -> Check {
    let lock = daemon::lock_path();
    if !lock.exists() {
        check(true, "daemon not running (no lock — interval saves off or not started)")
    } else if daemon::lockfile_names_live_anka(&lock) {
        check(true, format!("daemon running ({})", lock.display()))
    } else {
        check(
            false,
            format!("stale daemon lock ({}) — pid dead or not anka; the next daemon start reclaims it", lock.display()),
        )
    }
}

/// The `default` snapshot parses and its schema is supported (reuses the
/// schema gate). Absent = fine (nothing saved yet).
fn check_default_snapshot() -> Check {
    let json = store::snapshot_json(store::DEFAULT_SNAPSHOT);
    if !json.is_file() {
        return check(true, "no 'default' snapshot yet (nothing saved) — skipped");
    }
    let parsed = std::fs::read(&json)
        .map_err(anyhow::Error::from)
        .and_then(|data| model::parse_snapshot(&data));
    match parsed {
        Ok(s) => check(
            true,
            format!("'default' snapshot parses (schema {}, {} sessions)", s.schema, s.sessions.len()),
        ),
        Err(e) => check(false, format!("'default' snapshot unreadable: {e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_version_reads_the_package_version() {
        let toml = "[package]\nname = \"anka\"\nversion = \"1.2.3\"\nedition = \"2021\"\n";
        assert_eq!(cargo_version(toml), Some("1.2.3".to_string()));
    }

    #[test]
    fn cargo_version_none_when_absent() {
        assert_eq!(cargo_version("[package]\nname = \"x\"\n"), None);
        assert_eq!(cargo_version(""), None);
    }
}

//! Auto-save: event-driven hooks and the interval daemon.

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::capture;
use crate::config::Config;
use crate::store;
use crate::tmux;

/// Interval auto-save daemon: every `@anka-save-interval` minutes, save the
/// default snapshot; exit once the tmux server is gone. Single-instance via a
/// pidfile so repeated plugin loads don't spawn duplicate daemons.
pub fn run() -> Result<()> {
    let interval = interval_duration();
    if interval.is_zero() {
        return Ok(()); // disabled
    }
    let lock = lock_path();
    if another_daemon_running(&lock) {
        return Ok(());
    }
    write_pid(&lock);

    loop {
        thread::sleep(interval);
        if !tmux::server_running() {
            break;
        }
        let _ = capture::save(None);
    }

    let _ = fs::remove_file(&lock);
    Ok(())
}

fn lock_path() -> PathBuf {
    store::base_dir().join("daemon.lock")
}

/// True if the pid recorded in the lockfile is still alive.
fn another_daemon_running(lock: &Path) -> bool {
    fs::read_to_string(lock)
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .is_some_and(|pid| Path::new(&format!("/proc/{pid}")).exists())
}

fn write_pid(lock: &Path) {
    if let Some(parent) = lock.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(lock, std::process::id().to_string());
}

/// Whether a tmux hook event should trigger a save.
///
/// `session-closed` must NOT: by the time it fires the session is already gone,
/// so saving would re-capture the *smaller* session set and prune the just-closed
/// session from the snapshot. Worse, on logout/shutdown every session closes in
/// turn, so each `session-closed` shrinks the snapshot until only the
/// last-to-die sessions remain — silently losing the rest after a reboot.
pub fn should_save_on(event: &str) -> bool {
    event != "session-closed"
}

/// Event-driven save, invoked by tmux hooks (`client-detached`, …).
pub fn hook(event: &str) -> Result<()> {
    if should_save_on(event) {
        capture::save(None)?;
    }
    Ok(())
}

fn interval_duration() -> Duration {
    // A millisecond override keeps the daemon testable without minute-long waits.
    if let Ok(ms) = std::env::var("ANKA_DAEMON_INTERVAL_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            return Duration::from_millis(ms);
        }
    }
    Duration::from_secs(Config::load().save_interval_mins * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_closed_never_saves() {
        // Regression: closing a session must not prune it from the snapshot.
        assert!(!should_save_on("session-closed"));
        assert!(should_save_on("client-detached"));
    }
}

//! Auto-save: event-driven hooks and the interval daemon.

use anyhow::Result;
use std::fs;
use std::io::Write;
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
    if tick_duration().is_zero() || interval_disabled() {
        return Ok(()); // disabled
    }
    let lock = lock_path();
    if !acquire_lock(&lock) {
        return Ok(()); // another live anka daemon owns this store
    }

    loop {
        thread::sleep(tick_duration());
        if !tmux::server_running() {
            break;
        }
        // Re-read `@anka-save-interval` every tick: setting it to 0 must stop
        // the daemon — and a changed interval must apply — without a server
        // restart. (A shorter interval takes effect after the current nap.)
        if interval_disabled() {
            break;
        }
        let _ = capture::save(None);
    }

    let _ = fs::remove_file(&lock);
    Ok(())
}

pub(crate) fn lock_path() -> PathBuf {
    store::base_dir().join("daemon.lock")
}

/// Take the single-instance pidfile with `create_new` (O_EXCL), writing our
/// pid into it. If the file already exists, it only counts as "another daemon
/// running" when its pid names a *live process whose comm is `anka`* — a bare
/// existence/liveness check would let a kernel-recycled pid (now some
/// unrelated program) silently disable interval saves forever. A stale file
/// is removed and the create retried once.
///
/// Residual race (accepted, plugin-load only): two simultaneous plugin loads
/// that both find the same stale file can both remove it and re-create;
/// `create_new` serialises the create itself, but the loser's `remove_file`
/// may briefly delete the winner's fresh lock, letting one duplicate daemon
/// slip through until the next server restart. That beats the old
/// check-then-write gap, which had the same window *plus* the pid-reuse
/// false positive.
fn acquire_lock(lock: &Path) -> bool {
    if let Some(parent) = lock.parent() {
        let _ = fs::create_dir_all(parent);
    }
    for _ in 0..2 {
        match fs::OpenOptions::new().write(true).create_new(true).open(lock) {
            Ok(mut f) => {
                let _ = f.write_all(std::process::id().to_string().as_bytes());
                return true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if lockfile_names_live_anka(lock) {
                    return false;
                }
                let _ = fs::remove_file(lock); // stale → retry once
            }
            Err(_) => return false,
        }
    }
    false
}

/// True only when the pidfile's pid is a live process whose `/proc/<pid>/comm`
/// is `anka` (i.e. really one of us, not a recycled pid). Also used by
/// `anka doctor` to classify the lock as live vs stale.
pub(crate) fn lockfile_names_live_anka(lock: &Path) -> bool {
    let Some(pid) = fs::read_to_string(lock)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
    else {
        return false; // unreadable/garbage pidfile is stale by definition
    };
    fs::read_to_string(format!("/proc/{pid}/comm"))
        .map(|comm| comm.trim() == "anka")
        .unwrap_or(false)
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

/// How long to nap between saves. The env override only shortens the *nap*
/// for tests; enable/disable always follows the live tmux option (see `run`).
fn tick_duration() -> Duration {
    // A millisecond override keeps the daemon testable without minute-long waits.
    if let Ok(ms) = std::env::var("ANKA_DAEMON_INTERVAL_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            return Duration::from_millis(ms);
        }
    }
    Duration::from_secs(Config::load().save_interval_mins * 60)
}

/// `@anka-save-interval` == 0 → interval autosave off.
fn interval_disabled() -> bool {
    Config::load().save_interval_mins == 0
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

    fn scratch_lock(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anka-daemon-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("daemon.lock")
    }

    #[test]
    fn acquire_lock_takes_a_fresh_lock_and_records_our_pid() {
        let lock = scratch_lock("fresh");
        assert!(acquire_lock(&lock));
        assert_eq!(
            fs::read_to_string(&lock).unwrap().trim(),
            std::process::id().to_string()
        );
        fs::remove_dir_all(lock.parent().unwrap()).ok();
    }

    #[test]
    fn acquire_lock_steals_a_dead_pid_lock() {
        let lock = scratch_lock("dead");
        // u32::MAX is far past Linux's pid_max ceiling: never a live pid.
        fs::write(&lock, u32::MAX.to_string()).unwrap();
        assert!(acquire_lock(&lock), "a dead pid's lock must be stealable");
        fs::remove_dir_all(lock.parent().unwrap()).ok();
    }

    #[test]
    fn acquire_lock_steals_a_recycled_pid_lock() {
        let lock = scratch_lock("recycled");
        // Our own (test-runner) pid is live but its comm is NOT `anka` —
        // exactly the pid-reuse shape that used to disable saves forever.
        fs::write(&lock, std::process::id().to_string()).unwrap();
        assert!(
            acquire_lock(&lock),
            "a live-but-not-anka pid must be treated as stale (pid reuse)"
        );
        fs::remove_dir_all(lock.parent().unwrap()).ok();
    }

    #[test]
    fn acquire_lock_treats_garbage_pidfile_as_stale() {
        let lock = scratch_lock("garbage");
        fs::write(&lock, "not-a-pid").unwrap();
        assert!(acquire_lock(&lock));
        fs::remove_dir_all(lock.parent().unwrap()).ok();
    }
}

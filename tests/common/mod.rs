//! Shared helpers for integration tests that drive a throwaway tmux server.
#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Child, Command, Output};
use std::time::{Duration, Instant};

pub fn has_tmux() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn tmux_raw(socket: &str, args: &[&str]) -> Output {
    // `-f /dev/null` keeps the test server hermetic: a new tmux server would
    // otherwise source the user's tmux.conf (loading tmux-anka, auto-restoring
    // their real sessions into the test server).
    Command::new("tmux")
        .args(["-f", "/dev/null", "-L", socket])
        .args(args)
        .output()
        .expect("spawn tmux")
}

pub fn tmux(socket: &str, args: &[&str]) -> String {
    let o = tmux_raw(socket, args);
    assert!(
        o.status.success(),
        "tmux -L {socket} {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout)
        .trim_end_matches('\n')
        .to_string()
}

pub fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

pub fn sessions(socket: &str) -> Vec<String> {
    tmux(socket, &["list-sessions", "-F", "#{session_name}"])
        .lines()
        .map(String::from)
        .collect()
}

pub fn pane_count(socket: &str, target: &str) -> usize {
    tmux(socket, &["list-panes", "-t", target, "-F", "#{pane_id}"])
        .lines()
        .count()
}

pub fn window_count(socket: &str, session: &str) -> usize {
    tmux(socket, &["list-windows", "-t", session, "-F", "#{window_index}"])
        .lines()
        .count()
}

/// Block until `child` exits or `timeout` elapses; kills it on timeout.
pub fn wait_for_exit(child: &mut Child, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return true,
        }
    }
    let _ = child.kill();
    false
}

/// Owns a tmux socket + temp anka dir; cleans both up on drop.
pub struct Server {
    pub socket: String,
    pub dir: PathBuf,
    pub socket_path: String,
    pub pid: String,
}

impl Server {
    pub fn start(tag: &str) -> Self {
        // A per-run-unique token (nanos) avoids colliding with leaked servers
        // from earlier runs when the OS reuses our pid.
        let uniq = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket = format!("anka-it-{uniq}-{tag}");
        let dir = std::env::temp_dir().join(format!("anka-it-{uniq}-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        tmux_raw(&socket, &["kill-server"]); // ignore if none

        // The "scratch" session keeps the server alive across kill-session calls.
        tmux(&socket, &["new-session", "-d", "-s", "scratch", "-x", "200", "-y", "50"]);
        tmux(&socket, &["set-option", "-g", "@anka-dir", dir.to_str().unwrap()]);

        let socket_path = tmux(&socket, &["display-message", "-p", "#{socket_path}"]);
        let pid = tmux(&socket, &["display-message", "-p", "#{pid}"]);
        Server { socket, dir, socket_path, pid }
    }

    pub fn tmux(&self, args: &[&str]) -> String {
        tmux(&self.socket, args)
    }

    /// A `Command` for the anka binary, pointed at this server.
    pub fn anka_command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_anka"));
        c.args(args)
            .env("TMUX", format!("{},{},0", self.socket_path, self.pid));
        c
    }

    pub fn anka(&self, args: &[&str]) -> Output {
        self.anka_command(args).output().expect("spawn anka")
    }

    pub fn snapshot_json(&self, name: &str) -> PathBuf {
        self.dir.join("snapshots").join(name).join("snapshot.json")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        tmux_raw(&self.socket, &["kill-server"]);
        let _ = std::fs::remove_file(&self.socket_path); // stale socket file
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

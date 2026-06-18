//! Integration tests for `anka restore`, driving a throwaway tmux server.
//!
//! Each test uses its own `-L` socket and `@anka-dir`, so they can run in
//! parallel. Skipped automatically when `tmux` is not installed.

use std::path::PathBuf;
use std::process::{Command, Output};

fn has_tmux() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn tmux_raw(socket: &str, args: &[&str]) -> Output {
    Command::new("tmux")
        .arg("-L")
        .arg(socket)
        .args(args)
        .output()
        .expect("spawn tmux")
}

fn tmux(socket: &str, args: &[&str]) -> String {
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

fn anka(socket_path: &str, pid: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_anka"))
        .args(args)
        .env("TMUX", format!("{socket_path},{pid},0"))
        .output()
        .expect("spawn anka")
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn sessions(socket: &str) -> Vec<String> {
    tmux(socket, &["list-sessions", "-F", "#{session_name}"])
        .lines()
        .map(String::from)
        .collect()
}

fn pane_count(socket: &str, target: &str) -> usize {
    tmux(socket, &["list-panes", "-t", target, "-F", "#{pane_id}"])
        .lines()
        .count()
}

fn window_count(socket: &str, session: &str) -> usize {
    tmux(socket, &["list-windows", "-t", session, "-F", "#{window_index}"])
        .lines()
        .count()
}

/// A test fixture that owns a tmux socket + a temp anka dir and cleans up.
struct Server {
    socket: String,
    dir: PathBuf,
    socket_path: String,
    pid: String,
}

impl Server {
    fn start(tag: &str) -> Self {
        let socket = format!("anka-it-{}-{}", std::process::id(), tag);
        let dir = std::env::temp_dir().join(format!("anka-it-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        tmux_raw(&socket, &["kill-server"]); // ignore if none

        // A "scratch" session keeps the server alive across kill-session calls.
        tmux(&socket, &["new-session", "-d", "-s", "scratch", "-x", "200", "-y", "50"]);
        tmux(&socket, &["set-option", "-g", "@anka-dir", dir.to_str().unwrap()]);

        let socket_path = tmux(&socket, &["display-message", "-p", "#{socket_path}"]);
        let pid = tmux(&socket, &["display-message", "-p", "#{pid}"]);
        Server { socket, dir, socket_path, pid }
    }

    fn tmux(&self, args: &[&str]) -> String {
        tmux(&self.socket, args)
    }

    fn anka(&self, args: &[&str]) -> Output {
        anka(&self.socket_path, &self.pid, args)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        tmux_raw(&self.socket, &["kill-server"]);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn restore_rebuilds_session_tree() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("tree");

    // Build a known tree: alpha has 2 windows; its 2nd window has 2 panes.
    s.tmux(&["new-session", "-d", "-s", "alpha", "-x", "200", "-y", "50"]);
    s.tmux(&["new-window", "-t", "alpha", "-n", "editor"]);
    s.tmux(&["split-window", "-t", "alpha:editor", "-h"]);
    s.tmux(&["new-session", "-d", "-s", "beta", "-x", "200", "-y", "50"]);

    let out = s.anka(&["save", "snap"]);
    assert!(out.status.success(), "save failed: {}", String::from_utf8_lossy(&out.stderr));

    // Tear the sessions down (scratch keeps the server alive).
    s.tmux(&["kill-session", "-t", "alpha"]);
    s.tmux(&["kill-session", "-t", "beta"]);
    assert!(!sessions(&s.socket).contains(&"alpha".to_string()));

    let out = s.anka(&["restore", "snap"]);
    assert!(out.status.success(), "restore failed: {}", String::from_utf8_lossy(&out.stderr));

    let names = sessions(&s.socket);
    assert!(names.contains(&"alpha".to_string()), "alpha not restored: {names:?}");
    assert!(names.contains(&"beta".to_string()), "beta not restored: {names:?}");
    assert_eq!(window_count(&s.socket, "alpha"), 2, "alpha window count");
    assert_eq!(pane_count(&s.socket, "alpha:editor"), 2, "editor pane count");
}

#[test]
fn restore_preserves_pane_cwd() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("cwd");
    s.tmux(&["new-session", "-d", "-s", "work", "-c", "/tmp", "-x", "200", "-y", "50"]);

    let out = s.anka(&["save", "snap"]);
    assert!(out.status.success(), "save failed: {}", err(&out));

    s.tmux(&["kill-session", "-t", "work"]);
    let out = s.anka(&["restore", "snap"]);
    assert!(out.status.success(), "restore failed: {}", err(&out));

    let cwd = s.tmux(&["display-message", "-p", "-t", "work", "#{pane_current_path}"]);
    assert_eq!(cwd, "/tmp", "pane cwd not preserved");
}

#[test]
fn autostart_restores_once_per_server() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("autostart");
    s.tmux(&["set-option", "-g", "@anka-restore-on-start", "on"]);
    s.tmux(&["new-session", "-d", "-s", "gamma", "-x", "200", "-y", "50"]);

    // Save the default snapshot (becomes `last`).
    let out = s.anka(&["save"]);
    assert!(out.status.success(), "save failed: {}", err(&out));

    s.tmux(&["kill-session", "-t", "gamma"]);
    assert!(!sessions(&s.socket).contains(&"gamma".to_string()));

    // First autostart restores and marks the server.
    let out = s.anka(&["autostart"]);
    assert!(out.status.success(), "autostart failed: {}", err(&out));
    assert!(
        sessions(&s.socket).contains(&"gamma".to_string()),
        "gamma not restored by autostart"
    );
    assert!(
        !s.tmux(&["show-options", "-gqv", "@anka_restored"]).is_empty(),
        "@anka_restored not set"
    );

    // Second autostart must NOT restore again (once-per-server guard).
    s.tmux(&["kill-session", "-t", "gamma"]);
    let out = s.anka(&["autostart"]);
    assert!(out.status.success(), "autostart#2 failed: {}", err(&out));
    assert!(
        !sessions(&s.socket).contains(&"gamma".to_string()),
        "autostart re-restored despite the guard"
    );
}

//! Integration tests for `anka restore`, driving a throwaway tmux server.

mod common;
use common::*;

use std::time::Duration;

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
    assert!(out.status.success(), "save failed: {}", err(&out));

    s.tmux(&["kill-session", "-t", "alpha"]);
    s.tmux(&["kill-session", "-t", "beta"]);
    assert!(!sessions(&s.socket).contains(&"alpha".to_string()));

    let out = s.anka(&["restore", "snap"]);
    assert!(out.status.success(), "restore failed: {}", err(&out));

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
fn restore_keeps_pane_alive_when_process_command_exits() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("proc-exit");

    // A process pane whose command exits immediately (`true`). The pane must
    // survive as a shell — not vanish and take the session with it.
    let dir = s.dir.join("snapshots").join("rec");
    std::fs::create_dir_all(&dir).unwrap();
    let json = r#"{
      "schema":1,"anka_version":"t","saved_at":"t",
      "client":{"active_session":null,"last_session":null},
      "sessions":[{"name":"rec","windows":[{"index":1,"name":"w","active":true,
        "layout":"","automatic_rename":true,
        "panes":[{"index":1,"active":true,"title":"t","cwd":"/tmp","command":"ssh",
          "pid":0,"history_size":0,
          "restore":{"kind":"process","command":"true"}}]}]}]}"#;
    std::fs::write(dir.join("snapshot.json"), json).unwrap();

    let out = s.anka(&["restore", "rec"]);
    assert!(out.status.success(), "restore failed: {}", err(&out));
    std::thread::sleep(Duration::from_millis(400));

    let names = sessions(&s.socket);
    assert!(
        names.contains(&"rec".to_string()),
        "session vanished after the process command exited: {names:?}"
    );
    assert_eq!(pane_count(&s.socket, "rec"), 1, "pane did not survive");
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

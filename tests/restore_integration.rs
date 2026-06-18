//! Integration tests for `anka restore`, driving a throwaway tmux server.

mod common;
use common::*;

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

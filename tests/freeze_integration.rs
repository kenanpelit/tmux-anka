//! Integration tests for `anka freeze` / `anka up`, driving a throwaway server.

mod common;
use common::*;

#[test]
fn freeze_then_up_recreates_sessions() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("freeze");
    s.tmux(&["new-session", "-d", "-s", "proj", "-x", "200", "-y", "50"]);
    s.tmux(&["new-window", "-t", "proj", "-n", "logs"]);

    assert!(s.anka(&["save", "snap"]).status.success());

    let out = s.anka(&["freeze", "snap"]);
    assert!(out.status.success(), "freeze failed: {}", err(&out));
    let bp = s.dir.join("blueprints").join("snap.json");
    assert!(bp.is_file(), "blueprint json not written");

    s.tmux(&["kill-session", "-t", "proj"]);
    assert!(!sessions(&s.socket).contains(&"proj".to_string()));

    let out = s.anka(&["up", "snap"]);
    assert!(out.status.success(), "up failed: {}", err(&out));
    assert!(
        sessions(&s.socket).contains(&"proj".to_string()),
        "up did not recreate 'proj': {:?}",
        sessions(&s.socket)
    );
    assert_eq!(window_count(&s.socket, "proj"), 2, "window count after up");
}

#[test]
fn freeze_script_exports_runnable_file() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("freeze-script");
    s.tmux(&["new-session", "-d", "-s", "proj", "-x", "200", "-y", "50"]);
    assert!(s.anka(&["save", "snap"]).status.success());

    let out = s.anka(&["freeze", "snap", "--script"]);
    assert!(out.status.success(), "freeze --script failed: {}", err(&out));

    let sh = s.dir.join("blueprints").join("snap.sh");
    assert!(sh.is_file(), "script not written");
    let body = std::fs::read_to_string(&sh).unwrap();
    assert!(body.starts_with("#!/bin/sh"), "missing shebang");
    assert!(body.contains("tmux new-session -d -s proj"), "missing session cmd: {body}");
}

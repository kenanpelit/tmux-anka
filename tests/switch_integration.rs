//! Integration tests for `anka switch` (non-tty fallback), throwaway server.

mod common;
use common::*;

#[test]
fn switch_fallback_switches_to_chosen_live_session() {
    if !has_tmux() {
        eprintln!("skip");
        return;
    }
    let s = Server::start("sw-live");
    s.tmux(&["new-session", "-d", "-s", "alpha", "-x", "200", "-y", "50"]);
    s.tmux(&["new-session", "-d", "-s", "beta", "-x", "200", "-y", "50"]);
    let out = s.anka_stdin(&["switch"], "beta\n");
    assert!(out.status.success(), "{}", err(&out));
    // both still live (switching doesn't destroy anything)
    let live = sessions(&s.socket);
    assert!(live.contains(&"alpha".to_string()) && live.contains(&"beta".to_string()));
}

#[test]
fn switch_fallback_restores_snapshot_session() {
    if !has_tmux() {
        eprintln!("skip");
        return;
    }
    let s = Server::start("sw-snap");
    s.tmux(&["new-session", "-d", "-s", "gone", "-x", "200", "-y", "50"]);
    assert!(s.anka(&["save"]).status.success());
    s.tmux(&["kill-session", "-t", "gone"]);
    assert!(!sessions(&s.socket).contains(&"gone".to_string()));

    let out = s.anka_stdin(&["switch"], "gone\n");
    assert!(out.status.success(), "{}", err(&out));
    assert!(
        sessions(&s.socket).contains(&"gone".to_string()),
        "switch did not restore the snapshot session: {:?}",
        sessions(&s.socket)
    );
}

#[test]
fn switch_cancel_does_nothing() {
    if !has_tmux() {
        eprintln!("skip");
        return;
    }
    let s = Server::start("sw-cancel");
    s.tmux(&["new-session", "-d", "-s", "keep", "-x", "200", "-y", "50"]);
    let out = s.anka_stdin(&["switch"], "q\n");
    assert!(out.status.success(), "{}", err(&out));
    assert!(sessions(&s.socket).contains(&"keep".to_string()));
}

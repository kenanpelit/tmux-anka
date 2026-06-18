//! Integration tests for `anka session …`, driving a throwaway server.

mod common;
use common::*;

#[test]
fn session_new_creates_and_switches() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("sess-new");
    assert!(s.anka(&["session", "new", "work"]).status.success());
    assert!(sessions(&s.socket).contains(&"work".to_string()));
}

#[test]
fn session_rename_renames_current() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("sess-rn");
    s.tmux(&["new-session", "-d", "-s", "old", "-x", "200", "-y", "50"]);
    assert!(s.anka_in("old", &["session", "rename", "newname"]).status.success());
    let names = sessions(&s.socket);
    assert!(
        names.contains(&"newname".to_string()) && !names.contains(&"old".to_string()),
        "{names:?}"
    );
}

#[test]
fn session_kill_refuses_last_session() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("sess-kill1");
    // only "scratch" exists; killing it would drop the server → must refuse
    let out = s.anka(&["session", "kill"]);
    assert!(!out.status.success(), "kill should refuse the last session");
    assert!(sessions(&s.socket).contains(&"scratch".to_string()));
}

#[test]
fn session_kill_switches_then_kills() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("sess-kill2");
    s.tmux(&["new-session", "-d", "-s", "victim", "-x", "200", "-y", "50"]);
    assert!(s.anka_in("victim", &["session", "kill"]).status.success());
    assert!(!sessions(&s.socket).contains(&"victim".to_string()));
}

#[test]
fn session_promote_moves_pane_to_new_session() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("sess-prom");
    s.tmux(&["new-session", "-d", "-s", "src", "-x", "200", "-y", "50"]);
    s.tmux(&["split-window", "-t", "src"]);
    let out = s.anka_in("src", &["session", "promote", "promoted"]);
    assert!(out.status.success(), "promote failed: {}", err(&out));
    assert!(sessions(&s.socket).contains(&"promoted".to_string()));
}

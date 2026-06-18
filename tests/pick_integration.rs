//! Integration test for the `anka pick` picker, driving a throwaway server.

mod common;
use common::*;

#[test]
fn pick_restores_the_chosen_session() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("pick");
    // Sessions are stored name-sorted: "one" < "scratch" < "two", so #1 = "one".
    s.tmux(&["new-session", "-d", "-s", "one", "-x", "200", "-y", "50"]);
    s.tmux(&["new-session", "-d", "-s", "two", "-x", "200", "-y", "50"]);
    assert!(s.anka(&["save"]).status.success());

    s.tmux(&["kill-session", "-t", "one"]);
    s.tmux(&["kill-session", "-t", "two"]);
    assert!(!sessions(&s.socket).contains(&"one".to_string()));

    // Choose entry #1 ("one"); the other listed session must NOT be restored.
    let out = s.anka_stdin(&["pick"], "1\n");
    assert!(out.status.success(), "pick failed: {}", err(&out));

    let live = sessions(&s.socket);
    assert!(live.contains(&"one".to_string()), "pick did not restore 'one': {live:?}");
    assert!(!live.contains(&"two".to_string()), "pick restored too much: {live:?}");
}

#[test]
fn pick_cancel_restores_nothing() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("pick-cancel");
    s.tmux(&["new-session", "-d", "-s", "alpha", "-x", "200", "-y", "50"]);
    assert!(s.anka(&["save"]).status.success());
    s.tmux(&["kill-session", "-t", "alpha"]);

    let out = s.anka_stdin(&["pick"], "q\n");
    assert!(out.status.success(), "pick failed: {}", err(&out));
    assert!(!sessions(&s.socket).contains(&"alpha".to_string()), "cancel restored a session");
}

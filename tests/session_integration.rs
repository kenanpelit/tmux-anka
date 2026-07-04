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

// ── --prompt mode (non-tty stdin fallback; the popup path uses raw mode) ────

#[test]
fn session_new_prompt_reads_the_name_from_stdin() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("sess-prompt-new");
    let out = s.anka_stdin(&["session", "new", "--prompt"], "typed\n");
    assert!(out.status.success(), "prompted new failed: {}", err(&out));
    assert!(sessions(&s.socket).contains(&"typed".to_string()));
}

#[test]
fn session_new_prompt_empty_input_cancels_silently() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("sess-prompt-cancel");
    let before = sessions(&s.socket);
    let out = s.anka_stdin(&["session", "new", "--prompt"], "\n");
    assert!(out.status.success(), "cancel must exit 0: {}", err(&out));
    assert_eq!(sessions(&s.socket), before, "cancel must not create anything");
}

#[test]
fn session_switch_prompt_uses_the_typed_name() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("sess-prompt-sw");
    // A missing name must fail loudly — proof the prompt input reached switch.
    let out = s.anka_stdin(&["session", "switch", "--prompt"], "no-such-session\n");
    assert!(!out.status.success(), "switch to a missing session must fail");
    assert!(err(&out).contains("not found"), "{}", err(&out));
    // Empty input cancels silently instead.
    let out = s.anka_stdin(&["session", "switch", "--prompt"], "\n");
    assert!(out.status.success(), "cancel must exit 0: {}", err(&out));
}

#[test]
fn session_new_without_name_or_prompt_is_a_usage_error() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("sess-noname");
    let out = s.anka(&["session", "new"]);
    assert!(!out.status.success(), "bare `session new` must be rejected");
}

#[test]
fn session_new_switches_to_existing_instead_of_failing() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("sess-new-dup");
    s.tmux(&["new-session", "-d", "-s", "dup", "-x", "200", "-y", "50"]);
    // Colliding name (the zoxide-Enter shape): must succeed, not
    // "duplicate session".
    let out = s.anka(&["session", "new", "dup"]);
    assert!(out.status.success(), "existing name must switch, not fail: {}", err(&out));
    assert_eq!(
        sessions(&s.socket).iter().filter(|n| n.as_str() == "dup").count(),
        1
    );
}

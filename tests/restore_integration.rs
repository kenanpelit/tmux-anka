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

    // The shell's rc can transiently report the server's cwd before settling
    // into the `-c` dir; wait for it to settle so we save a realistic state.
    for _ in 0..40 {
        if s.tmux(&["display-message", "-p", "-t", "work", "#{pane_current_path}"]) == "/tmp" {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let out = s.anka(&["save", "snap"]);
    assert!(out.status.success(), "save failed: {}", err(&out));

    s.tmux(&["kill-session", "-t", "work"]);
    let out = s.anka(&["restore", "snap"]);
    assert!(out.status.success(), "restore failed: {}", err(&out));

    // Same as on save: the restored shell's rc can transiently report the
    // server's cwd before settling into the `-c` dir, so poll for settle rather
    // than reading the (racy) value the instant restore returns.
    let mut cwd = String::new();
    for _ in 0..40 {
        cwd = s.tmux(&["display-message", "-p", "-t", "work", "#{pane_current_path}"]);
        if cwd == "/tmp" {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
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
fn restore_overwrite_replaces_live_session() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("overwrite");
    s.tmux(&["new-session", "-d", "-s", "work", "-x", "200", "-y", "50"]);
    s.tmux(&["new-window", "-t", "work", "-n", "extra"]);
    let out = s.anka(&["save", "snap"]);
    assert!(out.status.success(), "save failed: {}", err(&out));

    // Reshape the live session so a rebuild is distinguishable from a skip.
    s.tmux(&["kill-window", "-t", "work:extra"]);
    assert_eq!(window_count(&s.socket, "work"), 1);

    // Default (overwrite off): the live session must be skipped, untouched.
    let out = s.anka(&["restore", "snap"]);
    assert!(out.status.success(), "restore failed: {}", err(&out));
    assert_eq!(
        window_count(&s.socket, "work"),
        1,
        "overwrite off must never touch a live session"
    );

    // Overwrite on: the live session is killed and rebuilt from the snapshot.
    // (Previously `new-session` died with "duplicate session", aborting the
    // whole restore — the option could never work.)
    s.tmux(&["set-option", "-g", "@anka-restore-overwrite", "on"]);
    let out = s.anka(&["restore", "snap"]);
    assert!(out.status.success(), "restore with overwrite failed: {}", err(&out));
    assert_eq!(
        window_count(&s.socket, "work"),
        2,
        "live session must be replaced by the snapshot's shape"
    );
}

#[test]
fn restore_preserves_noncontiguous_window_indices() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("winidx");
    s.tmux(&["new-session", "-d", "-s", "idx", "-x", "200", "-y", "50"]); // window 0
    s.tmux(&["new-window", "-t", "idx"]); // window 1
    s.tmux(&["new-window", "-t", "idx"]); // window 2
    s.tmux(&["kill-window", "-t", "idx:0"]); // leaves 1,2 (renumber-windows off)

    let out = s.anka(&["save", "snap"]);
    assert!(out.status.success(), "save failed: {}", err(&out));
    s.tmux(&["kill-session", "-t", "idx"]);

    let out = s.anka(&["restore", "snap"]);
    assert!(out.status.success(), "restore failed: {}", err(&out));
    let idxs: Vec<String> = s
        .tmux(&["list-windows", "-t", "idx", "-F", "#{window_index}"])
        .lines()
        .map(String::from)
        .collect();
    assert_eq!(
        idxs,
        vec!["1", "2"],
        "windows must come back at their recorded indices, not renumbered 0,1"
    );
}

#[test]
fn restore_refuses_future_schema_snapshot() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("schema-gate");
    // Doctored snapshot from an imaginary newer anka: unknown schema, and a
    // shape this binary doesn't fully know. Must fail with the guidance
    // message, not a raw serde "missing field" error.
    let dir = s.dir.join("snapshots").join("future");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("snapshot.json"),
        r#"{"schema":99,"anka_version":"9.9.9","brand_new_field":{"x":1}}"#,
    )
    .unwrap();

    let out = s.anka(&["restore", "future"]);
    assert!(!out.status.success(), "future-schema restore must fail");
    let msg = err(&out);
    assert!(msg.contains("schema 99"), "must name the snapshot's schema: {msg}");
    assert!(msg.contains("upgrade anka or re-save"), "unhelpful error: {msg}");
    assert!(!msg.contains("missing field"), "raw serde error leaked: {msg}");
}

#[test]
fn path_escaping_snapshot_names_are_rejected() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("badnames");
    for bad in ["../evil", "a/b", ".hidden", "last"] {
        let out = s.anka(&["save", bad]);
        assert!(!out.status.success(), "save must reject the name {bad:?}");
        let out = s.anka(&["rm", bad]);
        assert!(!out.status.success(), "rm must reject the name {bad:?}");
        let out = s.anka(&["restore", bad]);
        assert!(!out.status.success(), "restore must reject the name {bad:?}");
    }
    // Nothing may have been written outside (or into) the store by the attempts.
    assert!(!s.dir.join("evil").exists());
    assert!(!s.dir.parent().unwrap().join("evil").exists());
}

#[test]
fn save_prunes_stale_pane_capture_files() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("prune");
    s.tmux(&["new-session", "-d", "-s", "pr", "-x", "200", "-y", "50"]);
    s.tmux(&["split-window", "-t", "pr"]);

    let out = s.anka(&["save", "snap"]);
    assert!(out.status.success(), "save failed: {}", err(&out));
    let panes_dir = s.dir.join("snapshots").join("snap").join("panes");
    let count = || std::fs::read_dir(&panes_dir).map(|rd| rd.count()).unwrap_or(0);
    // One capture file per live pane: scratch (1) + pr (2).
    assert_eq!(count(), 3, "expected one capture file per live pane");

    s.tmux(&["kill-pane", "-t", "pr"]); // pr's active pane dies → 2 panes remain overall
    let out = s.anka(&["save", "snap"]);
    assert!(out.status.success(), "second save failed: {}", err(&out));
    assert_eq!(count(), 2, "the dead pane's capture file must be pruned on re-save");
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

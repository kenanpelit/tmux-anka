//! Integration tests for the `anka daemon` interval auto-save.

mod common;
use common::*;

use std::process::Stdio;
use std::thread::sleep;
use std::time::Duration;

#[test]
fn daemon_saves_on_interval_then_exits_when_server_gone() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("daemon-save");
    s.tmux(&["new-session", "-d", "-s", "work", "-x", "200", "-y", "50"]);

    let mut child = s
        .anka_command(&["daemon"])
        .env("ANKA_DAEMON_INTERVAL_MS", "150")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");

    // Let a couple of ticks elapse.
    sleep(Duration::from_millis(700));
    assert!(
        s.snapshot_json("default").exists(),
        "daemon did not write a snapshot"
    );

    // When the server goes away, the daemon must exit.
    s.tmux(&["kill-server"]);
    assert!(
        wait_for_exit(&mut child, Duration::from_secs(3)),
        "daemon did not exit after the server was gone"
    );
}

#[test]
fn daemon_starts_despite_recycled_pid_lockfile() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("daemon-recycled");
    s.tmux(&["new-session", "-d", "-s", "work", "-x", "200", "-y", "50"]);

    // Plant a lockfile naming a pid that is live but NOT an anka process
    // (this test runner). The old existence-only liveness check treated any
    // recycled pid as "daemon already running", silently disabling interval
    // saves forever.
    std::fs::create_dir_all(&s.dir).unwrap();
    std::fs::write(s.dir.join("daemon.lock"), std::process::id().to_string()).unwrap();

    let mut child = s
        .anka_command(&["daemon"])
        .env("ANKA_DAEMON_INTERVAL_MS", "150")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");

    sleep(Duration::from_millis(700));
    assert!(
        s.snapshot_json("default").exists(),
        "daemon refused to start over a stale (recycled-pid) lockfile"
    );

    s.tmux(&["kill-server"]);
    assert!(wait_for_exit(&mut child, Duration::from_secs(3)));
}

#[test]
fn daemon_exits_when_interval_disabled_at_runtime() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("daemon-zero");
    s.tmux(&["new-session", "-d", "-s", "work", "-x", "200", "-y", "50"]);

    let mut child = s
        .anka_command(&["daemon"])
        .env("ANKA_DAEMON_INTERVAL_MS", "150")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");

    // Let it settle in its loop, then disable the interval live — the daemon
    // must notice on its next tick without any server restart.
    sleep(Duration::from_millis(500));
    s.tmux(&["set-option", "-g", "@anka-save-interval", "0"]);

    assert!(
        wait_for_exit(&mut child, Duration::from_secs(3)),
        "daemon must exit when @anka-save-interval is set to 0 at runtime"
    );
    assert!(
        !s.dir.join("daemon.lock").exists(),
        "daemon must remove its lockfile on exit"
    );
    s.tmux(&["kill-server"]);
}

#[test]
fn daemon_is_single_instance() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("daemon-single");
    s.tmux(&["new-session", "-d", "-s", "work", "-x", "200", "-y", "50"]);

    let spawn = || {
        s.anka_command(&["daemon"])
            .env("ANKA_DAEMON_INTERVAL_MS", "300")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon")
    };

    let mut first = spawn();
    sleep(Duration::from_millis(400)); // let the first take the lock
    let mut second = spawn();

    assert!(
        wait_for_exit(&mut second, Duration::from_secs(2)),
        "second daemon did not exit (no single-instance lock)"
    );
    assert!(
        first.try_wait().unwrap().is_none(),
        "first daemon exited unexpectedly"
    );

    s.tmux(&["kill-server"]);
    wait_for_exit(&mut first, Duration::from_secs(3));
}

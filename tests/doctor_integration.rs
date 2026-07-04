//! Integration tests for `anka doctor`, driving a throwaway tmux server.

mod common;
use common::*;

#[test]
fn doctor_passes_on_a_healthy_store() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("doctor-ok");
    assert!(s.anka(&["save"]).status.success(), "save failed");

    let out = s.anka(&["doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "doctor failed on a healthy store:\n{stdout}\n{}", err(&out));
    assert!(stdout.contains("PASS tmux on PATH"), "{stdout}");
    assert!(stdout.contains("store dir writable"), "{stdout}");
    assert!(stdout.contains("'default' snapshot parses"), "{stdout}");
    assert!(!stdout.contains("FAIL"), "no FAIL lines expected:\n{stdout}");
}

#[test]
fn doctor_reports_before_anything_saved() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("doctor-fresh");
    let out = s.anka(&["doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    // A fresh install is healthy: no snapshot yet is not a failure.
    assert!(out.status.success(), "doctor failed on a fresh store:\n{stdout}\n{}", err(&out));
    assert!(stdout.contains("no 'default' snapshot yet"), "{stdout}");
}

#[test]
fn doctor_fails_on_a_corrupt_default_snapshot() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("doctor-bad");
    let dir = s.dir.join("snapshots").join("default");
    std::fs::create_dir_all(&dir).unwrap();
    // Future-schema snapshot: the schema gate must flag it, and doctor exit 1.
    std::fs::write(dir.join("snapshot.json"), r#"{"schema":99,"anka_version":"9.9.9"}"#).unwrap();

    let out = s.anka(&["doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!out.status.success(), "doctor must exit 1 on a FAIL:\n{stdout}");
    assert!(stdout.contains("FAIL 'default' snapshot"), "{stdout}");
}

#[test]
fn doctor_flags_a_stale_daemon_lock() {
    if !has_tmux() {
        eprintln!("skip: no tmux");
        return;
    }
    let s = Server::start("doctor-lock");
    std::fs::create_dir_all(&s.dir).unwrap();
    // A live pid that is NOT an anka process (this test runner): stale.
    std::fs::write(s.dir.join("daemon.lock"), std::process::id().to_string()).unwrap();

    let out = s.anka(&["doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!out.status.success(), "stale lock must FAIL:\n{stdout}");
    assert!(stdout.contains("FAIL stale daemon lock"), "{stdout}");
}

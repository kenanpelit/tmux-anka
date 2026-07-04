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
    s.tmux(&["new-window", "-t", "proj", "-n", "logs"]);
    assert!(s.anka(&["save", "snap"]).status.success());

    let out = s.anka(&["freeze", "snap", "--script"]);
    assert!(out.status.success(), "freeze --script failed: {}", err(&out));

    let sh = s.dir.join("blueprints").join("snap.sh");
    assert!(sh.is_file(), "script not written");
    let body = std::fs::read_to_string(&sh).unwrap();
    assert!(body.starts_with("#!/bin/sh"), "missing shebang");
    // The session is created with its pane id captured, so all later
    // targeting is by id (immune to base-index / non-contiguous indices)…
    assert!(
        body.contains("w=$(tmux new-session -d -P -F '#{pane_id}' -s proj"),
        "missing id-captured session cmd: {body}"
    );
    // …the first window is moved to its recorded index…
    assert!(body.contains("tmux move-window -s \"$w\""), "missing first-window move: {body}");
    // …and later windows are created *at* their recorded indices.
    assert!(
        body.contains("-t proj:1 -n logs"),
        "second window not created at its recorded index: {body}"
    );
}

#[test]
fn freeze_script_recreates_noncontiguous_indices_when_run() {
    if !has_tmux() {
        eprintln!("skipping: tmux not installed");
        return;
    }
    let s = Server::start("freeze-script-run");
    s.tmux(&["new-session", "-d", "-s", "proj", "-x", "200", "-y", "50"]); // window 0
    s.tmux(&["new-window", "-t", "proj"]); // window 1
    s.tmux(&["new-window", "-t", "proj"]); // window 2
    s.tmux(&["kill-window", "-t", "proj:0"]); // leaves 1,2
    assert!(s.anka(&["save", "snap"]).status.success());
    assert!(s.anka(&["freeze", "snap", "--script"]).status.success());

    // Run the exported script on a *second* throwaway socket (its first
    // new-session boots a fresh server there): hermetic via `-f /dev/null`,
    // and immune to both the harness's live "scratch" session (a duplicate
    // would abort under `set -e`) and the old server's teardown race.
    // Also skip the trailing `tmux attach` — there is no terminal here.
    let run_socket = format!("{}-run", s.socket);
    let sh = s.dir.join("blueprints").join("snap.sh");
    // Only rewrite actual invocations (line starts and `$(tmux …)` captures):
    // a window may literally be *named* "tmux" (automatic-rename), and a
    // blanket "tmux " replace would corrupt `-n tmux -c …` arguments.
    let redirect = format!("tmux -f /dev/null -L {run_socket} ");
    let body = std::fs::read_to_string(&sh)
        .unwrap()
        .replace("\ntmux attach\n", "")
        .replace("$(tmux ", &format!("$({redirect}"))
        .replace("\ntmux ", &format!("\n{redirect}"));
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(&body)
        .output()
        .expect("run script");
    // `set -e`: any missed target aborts the script — the old index-targeted
    // form did exactly that under non-contiguous indices.
    assert!(
        out.status.success(),
        "exported script failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let idxs: Vec<String> = tmux(&run_socket, &["list-windows", "-t", "proj", "-F", "#{window_index}"])
        .lines()
        .map(String::from)
        .collect();
    tmux_raw(&run_socket, &["kill-server"]);
    assert_eq!(idxs, vec!["1", "2"], "script must recreate the recorded indices");
}

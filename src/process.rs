//! Resolve a pane's foreground command via Linux `/proc`.

use std::fs;

const SHELLS: &[&str] = &["bash", "zsh", "fish", "sh", "dash", "ksh", "tcsh", "csh"];

/// Best-effort foreground command line for a pane, given the pane's pid.
/// Descends to the youngest non-shell descendant. Returns `None` for shells.
pub fn foreground_command(pane_pid: i32) -> Option<String> {
    let mut pid = pane_pid;
    for _ in 0..16 {
        match newest_child(pid) {
            Some(child) => pid = child,
            None => break,
        }
    }
    let cmd = cmdline(pid)?;
    if SHELLS.contains(&base_name(&cmd)) {
        None
    } else {
        Some(cmd)
    }
}

/// The basename of the program in a command line.
pub fn base_name(cmd: &str) -> &str {
    let first = cmd.split_whitespace().next().unwrap_or("");
    first.rsplit('/').next().unwrap_or(first)
}

fn newest_child(pid: i32) -> Option<i32> {
    // /proc/<pid>/task/<pid>/children is a space-separated list of child pids.
    let content = fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).ok()?;
    content
        .split_whitespace()
        .filter_map(|s| s.parse::<i32>().ok())
        .max()
}

fn cmdline(pid: i32) -> Option<String> {
    let raw = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let s = raw
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

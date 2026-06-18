//! Resolve a pane's foreground command via Linux `/proc`.

use std::fs;

const SHELLS: &[&str] = &["bash", "zsh", "fish", "sh", "dash", "ksh", "tcsh", "csh"];

/// Best-effort foreground command line for a pane, given the pane's pid.
///
/// Returns the pane shell's direct child — the program the user actually ran
/// (nvim, ssh, npm, …) — **not** any deeper descendant that program spawned
/// (an editor's LSP server, a tool's worker, …). Returns `None` for a bare
/// shell.
pub fn foreground_command(pane_pid: i32) -> Option<String> {
    let child = newest_child(pane_pid)?;
    let cmd = cmdline(child)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::Duration;

    // shell -> python3 (the foreground program) -> sleep (a grandchild).
    // We must capture the program the user ran (python3), not a descendant it
    // happened to spawn (sleep) — the bug that captured an editor's LSP server.
    #[test]
    fn captures_foreground_program_not_its_grandchild() {
        let prog = "import subprocess,time; subprocess.Popen(['sleep','3']); time.sleep(3)";
        let mut shell = Command::new("sh")
            .arg("-c")
            // trailing `; true` stops sh from exec-replacing itself with python3
            .arg(format!("python3 -c \"{prog}\"; true"))
            .spawn()
            .expect("spawn sh");
        std::thread::sleep(Duration::from_millis(500));

        let got = foreground_command(shell.id() as i32);
        let _ = shell.kill();
        let _ = shell.wait();

        // The bug returned the bare grandchild ("sleep 3"); the fix returns the
        // program ("python3 -c ..."). (The python source mentions "sleep", so
        // assert on the leading program name rather than absence of "sleep".)
        let got = got.unwrap_or_default();
        assert!(
            got.starts_with("python3"),
            "expected the foreground program (python3 …), got: {got:?}"
        );
    }
}

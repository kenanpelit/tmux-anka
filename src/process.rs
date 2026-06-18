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
    reconstruct(&raw)
}

/// Rebuild a command line from NUL-separated argv bytes (the
/// `/proc/<pid>/cmdline` format), suitable for typing back into a shell.
///
/// Real argv (2+ elements) is rebuilt by shell-quoting each element, so the
/// shell re-parses the line into the *same* argv — without this an argument
/// containing shell metacharacters (the remote command in `ssh host "a || b"`)
/// would flatten to `ssh host a || b` and the local shell would (mis)interpret
/// the `||`/`&&`/spaces.
///
/// A single element is emitted verbatim: it is either a bare command or a
/// process that rewrote its own cmdline into one whole command string
/// (setproctitle — e.g. npm/node: `npm exec @anthropic-ai/claude-code`). That
/// is a command line, not one argument, so quoting it would collapse it into a
/// single bogus word ("no such file or directory").
fn reconstruct(raw: &[u8]) -> Option<String> {
    let parts: Vec<String> = raw
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    match parts.len() {
        0 => None,
        1 => parts.into_iter().next(),
        _ => Some(
            parts
                .iter()
                .map(|p| shell_quote(p))
                .collect::<Vec<_>>()
                .join(" "),
        ),
    }
}

/// POSIX shell-quote a single argument. Unquoted when it only contains
/// characters no shell treats specially; otherwise wrapped in single quotes
/// (with embedded `'` rendered as `'\''`).
fn shell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg.chars().all(is_shell_safe) {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('\'');
    for c in arg.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

fn is_shell_safe(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | '@' | ',' | '+')
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

    // Regression: an argument that itself contains shell metacharacters (the
    // remote command in `ssh host "a || b && c"`) must stay a single quoted
    // argument, so the restored command re-parses to the same argv instead of
    // letting the local shell interpret the `||`/`&&`.
    #[test]
    fn preserves_quoting_of_args_with_metacharacters() {
        let raw = b"ssh\0grid\0-t\0byobu has -t k || byobu new -s k && byobu a -t k\0";
        let got = reconstruct(raw).unwrap();
        assert_eq!(got, "ssh grid -t 'byobu has -t k || byobu new -s k && byobu a -t k'");
    }

    #[test]
    fn leaves_simple_args_unquoted() {
        let raw = b"npm\0run\0dev\0";
        assert_eq!(reconstruct(raw).unwrap(), "npm run dev");
    }

    #[test]
    fn escapes_embedded_single_quotes() {
        let raw = b"sh\0-c\0echo 'hi'\0";
        assert_eq!(reconstruct(raw).unwrap(), r#"sh -c 'echo '\''hi'\'''"#);
    }

    // Regression: npm/node rewrite argv into one space-joined string
    // (setproctitle), padded with trailing NULs. That single element is a whole
    // command line, not one argument — emit it verbatim, never quoted, or the
    // shell reads it as a single bogus filename ("no such file or directory").
    #[test]
    fn single_element_cmdline_is_verbatim() {
        let raw = b"npm exec @anthropic-ai/claude-code -r\0\0\0\0";
        assert_eq!(
            reconstruct(raw).unwrap(),
            "npm exec @anthropic-ai/claude-code -r"
        );
    }

    #[test]
    fn empty_cmdline_is_none() {
        assert_eq!(reconstruct(b""), None);
    }
}

//! `anka menu` — native action menu (replaces tmux-fzf).
//!
//! `prefix+F` runs a `run-shell` that captures `#{client_name}`/`#{session_name}`
//! (formats expand there, not in `display-popup -E`); anka then reopens itself in
//! a popup running the picker. The picker → category → target → action flow
//! targets the *invoking* client via `switch-client -c` / `command-prompt -t`.

use anyhow::Result;

use crate::picker;
use crate::process::shell_quote;
use crate::tmux;

/// Field separator used in our `tmux -F` formats (US, 0x1f).
const US: char = '\u{1f}';

const WIN_FMT: &str = "#{session_name}\u{1f}#{window_index}\u{1f}#{window_name}\u{1f}#{window_active}\u{1f}#{window_id}";
const PANE_FMT: &str = "#{session_name}\u{1f}#{window_index}\u{1f}#{pane_index}\u{1f}#{pane_id}\u{1f}#{pane_current_command}\u{1f}#{pane_current_path}";

#[derive(Debug, PartialEq)]
pub struct Item {
    pub label: String,
    pub target: String,
}

// ── Source parsers (pure) ────────────────────────────────────────────────────

pub fn parse_windows(out: &str) -> Vec<Item> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(US).collect();
            if f.len() < 5 {
                return None;
            }
            let star = if f[3] == "1" { " *" } else { "" };
            Some(Item {
                label: format!("{}:{} {}{}", f[0], f[1], f[2], star),
                target: f[4].to_string(),
            })
        })
        .collect()
}

pub fn parse_panes(out: &str) -> Vec<Item> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(US).collect();
            if f.len() < 6 {
                return None;
            }
            Some(Item {
                label: format!("{}:{}.{} {}  {}", f[0], f[1], f[2], f[4], f[5]),
                target: f[3].to_string(),
            })
        })
        .collect()
}

pub fn parse_processes(out: &str) -> Vec<Item> {
    out.lines()
        .filter_map(|l| {
            let (pid, args) = l.trim_start().split_once(char::is_whitespace)?;
            if pid.is_empty() || !pid.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            Some(Item {
                label: format!("{pid}  {}", args.trim()),
                target: pid.to_string(),
            })
        })
        .collect()
}

pub fn parse_commands(out: &str) -> Vec<Item> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|n| Item {
            label: n.to_string(),
            target: n.to_string(),
        })
        .collect()
}

pub fn parse_keys(out: &str, table: &str) -> Vec<Item> {
    let marker = format!("-T {table} ");
    out.lines()
        .filter_map(|l| {
            let after = &l[l.find(&marker)? + marker.len()..];
            let (key, cmd) = after.trim_start().split_once(char::is_whitespace)?;
            if key.is_empty() {
                return None;
            }
            Some(Item {
                label: format!("{table} {key}  →  {}", cmd.trim()),
                target: format!("{table}{US}{key}"),
            })
        })
        .collect()
}

// ── Action argv builders (pure) ──────────────────────────────────────────────

fn v(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

pub fn switch_argv(client: &str, id: &str) -> Vec<String> {
    v(&["switch-client", "-c", client, "-t", id])
}
pub fn kill_window_argv(id: &str) -> Vec<String> {
    v(&["kill-window", "-t", id])
}
pub fn kill_pane_argv(id: &str) -> Vec<String> {
    v(&["kill-pane", "-t", id])
}
pub fn rename_window_argv(client: &str, id: &str) -> Vec<String> {
    vec![
        "command-prompt".into(),
        "-t".into(),
        client.into(),
        "-p".into(),
        "rename window:".into(),
        format!("rename-window -t {id} '%%'"),
    ]
}
pub fn zoom_pane_argv(id: &str) -> Vec<String> {
    v(&["resize-pane", "-Z", "-t", id])
}
pub fn break_pane_argv(id: &str) -> Vec<String> {
    v(&["break-pane", "-s", id])
}
pub fn command_argv(client: &str, cmd: &str) -> Vec<String> {
    vec![
        "command-prompt".into(),
        "-t".into(),
        client.into(),
        "-p".into(),
        format!("{cmd}:"),
        format!("{cmd} %%"),
    ]
}
pub fn keybinding_argv(client: &str, target: &str, prefix: &str) -> Vec<String> {
    let (table, key) = target.split_once(US).unwrap_or(("root", target));
    if table == "prefix" {
        v(&["send-keys", "-t", client, prefix, key])
    } else {
        v(&["send-keys", "-t", client, key])
    }
}
pub fn kill_process_cmd(pid: &str) -> Vec<String> {
    v(&["kill", pid])
}

// ── Runtime flow ─────────────────────────────────────────────────────────────

pub fn run(run: bool, client: Option<&str>, session: Option<&str>) -> Result<()> {
    if !run {
        return launch_popup(client, session);
    }
    let client = client.unwrap_or("");
    let cats = ["command", "keybinding", "process", "window", "pane"];
    let labels: Vec<String> = cats.iter().map(|s| s.to_string()).collect();
    let Some(ci) = picker::pick(&labels, "menu")? else {
        return Ok(());
    };
    match cats[ci] {
        "command" => category_command(client),
        "keybinding" => category_keybinding(client),
        "process" => category_process(),
        "window" => category_window(client),
        "pane" => category_pane(client),
        _ => Ok(()),
    }
}

fn launch_popup(client: Option<&str>, session: Option<&str>) -> Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = format!("{} menu --run", exe.display());
    if let Some(c) = client {
        cmd.push_str(&format!(" --client {}", shell_quote(c)));
    }
    if let Some(s) = session {
        cmd.push_str(&format!(" --session {}", shell_quote(s)));
    }
    tmux::run_ok(&["display-popup", "-w", "60%", "-h", "50%", "-E", &cmd]);
    Ok(())
}

fn notify(msg: &str) {
    tmux::run_ok(&["display-message", msg]);
}

fn run_argv(argv: &[String]) {
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    tmux::run_ok(&refs);
}

fn labels_of(items: &[Item]) -> Vec<String> {
    items.iter().map(|i| i.label.clone()).collect()
}

fn pick_action(actions: &[&str]) -> Result<Option<usize>> {
    let labels: Vec<String> = actions.iter().map(|s| s.to_string()).collect();
    picker::pick(&labels, "action")
}

fn category_command(client: &str) -> Result<()> {
    let out = tmux::run(&["list-commands", "-F", "#{command_list_name}"]).unwrap_or_default();
    let items = parse_commands(&out);
    if items.is_empty() {
        notify("anka: no commands");
        return Ok(());
    }
    if let Some(i) = picker::pick(&labels_of(&items), "command")? {
        run_argv(&command_argv(client, &items[i].target));
    }
    Ok(())
}

fn category_keybinding(client: &str) -> Result<()> {
    let prefix = tmux::run(&["show-options", "-gv", "prefix"]).unwrap_or_else(|_| "C-b".into());
    let mut items = Vec::new();
    for table in ["prefix", "root"] {
        let out = tmux::run(&["list-keys", "-T", table]).unwrap_or_default();
        items.extend(parse_keys(&out, table));
    }
    if items.is_empty() {
        notify("anka: no key bindings");
        return Ok(());
    }
    if let Some(i) = picker::pick(&labels_of(&items), "keybinding")? {
        run_argv(&keybinding_argv(client, &items[i].target, &prefix));
    }
    Ok(())
}

fn category_process() -> Result<()> {
    let out = std::process::Command::new("ps")
        .args(["-eo", "pid=,args="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let items = parse_processes(&out);
    if items.is_empty() {
        notify("anka: no processes");
        return Ok(());
    }
    if let Some(i) = picker::pick(&labels_of(&items), "process (kill)")? {
        let argv = kill_process_cmd(&items[i].target);
        let _ = std::process::Command::new(&argv[0]).args(&argv[1..]).status();
    }
    Ok(())
}

fn category_window(client: &str) -> Result<()> {
    let out = tmux::run(&["list-windows", "-a", "-F", WIN_FMT]).unwrap_or_default();
    let items = parse_windows(&out);
    if items.is_empty() {
        notify("anka: no windows");
        return Ok(());
    }
    let Some(i) = picker::pick(&labels_of(&items), "window")? else {
        return Ok(());
    };
    let id = &items[i].target;
    let actions = ["switch", "kill", "rename"];
    if let Some(a) = pick_action(&actions)? {
        let argv = match actions[a] {
            "switch" => switch_argv(client, id),
            "kill" => kill_window_argv(id),
            "rename" => rename_window_argv(client, id),
            _ => return Ok(()),
        };
        run_argv(&argv);
    }
    Ok(())
}

fn category_pane(client: &str) -> Result<()> {
    let out = tmux::run(&["list-panes", "-a", "-F", PANE_FMT]).unwrap_or_default();
    let items = parse_panes(&out);
    if items.is_empty() {
        notify("anka: no panes");
        return Ok(());
    }
    let Some(i) = picker::pick(&labels_of(&items), "pane")? else {
        return Ok(());
    };
    let id = &items[i].target;
    let actions = ["switch", "kill", "zoom", "break"];
    if let Some(a) = pick_action(&actions)? {
        let argv = match actions[a] {
            "switch" => switch_argv(client, id),
            "kill" => kill_pane_argv(id),
            "zoom" => zoom_pane_argv(id),
            "break" => break_pane_argv(id),
            _ => return Ok(()),
        };
        run_argv(&argv);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(fields: &[&str]) -> String {
        fields.join(&US.to_string())
    }

    #[test]
    fn windows_label_and_target() {
        let out = format!(
            "{}\n{}",
            line(&["KENP", "1", "kenp", "0", "@4"]),
            line(&["KENP", "2", "edit", "1", "@5"]),
        );
        let items = parse_windows(&out);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], Item { label: "KENP:1 kenp".into(), target: "@4".into() });
        assert_eq!(items[1], Item { label: "KENP:2 edit *".into(), target: "@5".into() });
    }

    #[test]
    fn panes_target_is_pane_id() {
        let out = line(&["KENP", "2", "1", "%7", "nvim", "/home/kenan"]);
        let items = parse_panes(&out);
        assert_eq!(items[0].target, "%7");
        assert!(items[0].label.starts_with("KENP:2.1 nvim"));
    }

    #[test]
    fn processes_pid_first_token() {
        let out = "  1234 /usr/bin/foo --bar\n   56 zsh\nheader junk\n";
        let items = parse_processes(out);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], Item { label: "1234  /usr/bin/foo --bar".into(), target: "1234".into() });
        assert_eq!(items[1].target, "56");
    }

    #[test]
    fn commands_names() {
        let items = parse_commands("new-window\nkill-pane\n\n");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], Item { label: "new-window".into(), target: "new-window".into() });
    }

    #[test]
    fn keys_table_and_key() {
        let out = "bind-key    -T prefix c    new-window\nbind-key -r -T prefix C-Up resize-pane -U";
        let items = parse_keys(out, "prefix");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].target, format!("prefix{US}c"));
        assert_eq!(items[1].target, format!("prefix{US}C-Up"));
        assert!(items[0].label.contains("new-window"));
    }

    #[test]
    fn switch_targets_client_and_id() {
        assert_eq!(switch_argv("c0", "@5"), vec!["switch-client", "-c", "c0", "-t", "@5"]);
    }

    #[test]
    fn window_kill_and_rename() {
        assert_eq!(kill_window_argv("@5"), vec!["kill-window", "-t", "@5"]);
        assert_eq!(
            rename_window_argv("c0", "@5"),
            vec!["command-prompt", "-t", "c0", "-p", "rename window:", "rename-window -t @5 '%%'"]
        );
    }

    #[test]
    fn pane_actions() {
        assert_eq!(kill_pane_argv("%7"), vec!["kill-pane", "-t", "%7"]);
        assert_eq!(zoom_pane_argv("%7"), vec!["resize-pane", "-Z", "-t", "%7"]);
        assert_eq!(break_pane_argv("%7"), vec!["break-pane", "-s", "%7"]);
    }

    #[test]
    fn command_bakes_name_into_template() {
        assert_eq!(
            command_argv("c0", "new-window"),
            vec!["command-prompt", "-t", "c0", "-p", "new-window:", "new-window %%"]
        );
    }

    #[test]
    fn keybinding_prefix_vs_root() {
        let pfx = format!("prefix{US}c");
        assert_eq!(keybinding_argv("c0", &pfx, "C-b"), vec!["send-keys", "-t", "c0", "C-b", "c"]);
        let root = format!("root{US}F1");
        assert_eq!(keybinding_argv("c0", &root, "C-b"), vec!["send-keys", "-t", "c0", "F1"]);
    }

    #[test]
    fn kill_process_is_sigterm() {
        assert_eq!(kill_process_cmd("1234"), vec!["kill", "1234"]);
    }
}

//! `@anka-*` configuration, read from tmux global options.

use crate::tmux;

#[allow(dead_code)] // some fields are consumed by restore/daemon (later versions)
pub struct Config {
    pub capture_pane_contents: bool,
    pub restore_processes: Vec<String>,
    pub strategy_nvim: String,
    pub save_interval_mins: u64,
    pub restore_on_start: bool,
    pub restore_overwrite: bool,
}

fn opt_or(name: &str, default: &str) -> String {
    let v = tmux::global_option(name);
    if v.is_empty() {
        default.to_string()
    } else {
        v
    }
}

fn opt_bool(name: &str, default: bool) -> bool {
    match tmux::global_option(name).as_str() {
        "" => default,
        "on" | "1" | "true" | "yes" => true,
        _ => false,
    }
}

impl Config {
    pub fn load() -> Self {
        Config {
            capture_pane_contents: opt_bool("@anka-capture-pane-contents", true),
            restore_processes: opt_or(
                "@anka-restore-processes",
                "ssh psql mysql sqlite3 npm yarn nvim",
            )
            .split_whitespace()
            .map(String::from)
            .collect(),
            strategy_nvim: opt_or("@anka-strategy-nvim", "session"),
            save_interval_mins: opt_or("@anka-save-interval", "10").parse().unwrap_or(10),
            restore_on_start: opt_bool("@anka-restore-on-start", true),
            restore_overwrite: opt_bool("@anka-restore-overwrite", false),
        }
    }
}

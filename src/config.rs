//! `@anka-*` configuration, read from tmux global options.

use crate::tmux;

pub struct Config {
    pub capture_pane_contents: bool,
    pub restore_processes: Vec<String>,
    pub strategy_nvim: String,
    pub save_interval_mins: u64,
    pub restore_on_start: bool,
    pub restore_overwrite: bool,
    pub switch_preview: bool,
    pub zoxide: bool,
    /// How many prior versions of a saved snapshot to keep as `.1`, `.2`, …
    /// generations before overwriting it (`@anka-save-generations`, `0` disables).
    pub save_generations: u32,
    /// Scrollback lines captured per pane (`@anka-history-limit`, `0` = unlimited).
    pub history_limit: u64,
    /// Replay self-retitled (single-element setproctitle) command lines even
    /// when they contain shell metacharacters (`@anka-restore-verbatim`,
    /// default off — see `capture::classify_restore`).
    pub restore_verbatim: bool,
}

fn opt_or(name: &str, default: &str) -> String {
    let v = tmux::global_option(name);
    if v.is_empty() {
        default.to_string()
    } else {
        v
    }
}

/// Parse a tmux boolean option value: the truthy set {on, yes, true, 1},
/// case-insensitive (`On`/`ON` count). Empty (unset) → `default`.
pub fn parse_bool(v: &str, default: bool) -> bool {
    if v.is_empty() {
        return default;
    }
    matches!(v.to_ascii_lowercase().as_str(), "on" | "1" | "true" | "yes")
}

fn opt_bool(name: &str, default: bool) -> bool {
    parse_bool(&tmux::global_option(name), default)
}

/// Parse a numeric option value. `(value, failed)`: empty (unset) is the
/// default and NOT a failure; a set-but-unparsable value falls back to the
/// default with `failed = true` so the caller can warn instead of silently
/// swallowing a typo like `@anka-save-interval "ten"`.
pub fn parse_num<T: std::str::FromStr + Copy>(raw: &str, default: T) -> (T, bool) {
    if raw.is_empty() {
        return (default, false);
    }
    match raw.trim().parse() {
        Ok(v) => (v, false),
        Err(_) => (default, true),
    }
}

/// One warning per process: `Config::load` runs on every daemon tick, and a
/// bad option must not turn into a display-message drumbeat.
static WARNED_BAD_OPTION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn opt_num<T: std::str::FromStr + Copy>(name: &str, default_raw: &str, default: T) -> T {
    let raw = opt_or(name, default_raw);
    let (v, failed) = parse_num(&raw, default);
    if failed && !WARNED_BAD_OPTION.swap(true, std::sync::atomic::Ordering::SeqCst) {
        crate::notify::error(&format!("invalid {name} value {raw:?} — using {default_raw}"));
    }
    v
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
            save_interval_mins: opt_num("@anka-save-interval", "10", 10),
            restore_on_start: opt_bool("@anka-restore-on-start", true),
            restore_overwrite: opt_bool("@anka-restore-overwrite", false),
            switch_preview: opt_bool("@anka-switch-preview", true),
            zoxide: opt_bool("@anka-zoxide", true),
            save_generations: opt_num("@anka-save-generations", "3", 3),
            history_limit: opt_num("@anka-history-limit", "2000", 2000),
            restore_verbatim: opt_bool("@anka-restore-verbatim", false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bool_truthy_set_is_case_insensitive() {
        // Regression: "On"/"ON" used to be treated as false.
        for v in ["on", "On", "ON", "yes", "Yes", "true", "TRUE", "1"] {
            assert!(parse_bool(v, false), "{v:?} must be true");
        }
    }

    #[test]
    fn parse_bool_falsy_and_garbage_are_false() {
        for v in ["off", "Off", "0", "false", "no", "junk"] {
            assert!(!parse_bool(v, true), "{v:?} must be false");
        }
    }

    #[test]
    fn parse_bool_empty_is_the_default() {
        assert!(parse_bool("", true));
        assert!(!parse_bool("", false));
    }

    #[test]
    fn parse_num_empty_is_default_without_failure() {
        assert_eq!(parse_num::<u64>("", 10), (10, false));
    }

    #[test]
    fn parse_num_valid_value_parses() {
        assert_eq!(parse_num::<u64>("7", 10), (7, false));
        assert_eq!(parse_num::<u32>(" 4 ", 3), (4, false));
    }

    #[test]
    fn parse_num_garbage_falls_back_and_flags_failure() {
        // A set-but-invalid interval must be *reported*, not silently defaulted.
        assert_eq!(parse_num::<u64>("ten", 10), (10, true));
        assert_eq!(parse_num::<u32>("-1", 3), (3, true));
    }
}

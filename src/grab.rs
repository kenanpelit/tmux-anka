//! `anka grab` — extract tokens from the pane and copy/paste/open (replaces
//! extrakto). Filters are cycled with Tab; actions: Enter=copy, ^v=paste, ^o=open.
//! Two-step launch like `anka url`: capture the pane, reopen in a popup picker.

use std::collections::HashSet;

use anyhow::Result;

use crate::picker::{self, Hit};

pub const FILTERS: &[&str] = &["all", "url", "path", "word", "line"];

const TRIM: &[char] = &['(', ')', '[', ']', '{', '}', '<', '>', '"', '\'', ',', ';', ':', '!', '?'];

/// Non-empty, de-duplicated whole lines (trimmed).
pub fn parse_lines(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| seen.insert(l.to_string()))
        .map(String::from)
        .collect()
}

/// Whitespace tokens of length ≥ 2, surrounding punctuation trimmed, deduped.
pub fn parse_words(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for tok in text.split_whitespace() {
        let w = tok.trim_matches(TRIM);
        if w.chars().count() >= 2 && seen.insert(w.to_string()) {
            out.push(w.to_string());
        }
    }
    out
}

fn is_path(s: &str) -> bool {
    if s.len() < 2 {
        return false;
    }
    s.starts_with('/')
        || s.starts_with("~/")
        || s.starts_with("./")
        || s.starts_with("../")
        || (s.contains('/') && s.rsplit('/').next().is_some_and(|f| f.contains('.')))
}

/// Filesystem-path-looking tokens, deduped.
pub fn parse_paths(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for tok in text.split_whitespace() {
        let t = tok.trim_matches(TRIM);
        if is_path(t) && seen.insert(t.to_string()) {
            out.push(t.to_string());
        }
    }
    out
}

/// Tokens for a filter. `all` = url ++ path ++ word, deduped in that priority.
pub fn tokens_for(filter: &str, text: &str) -> Vec<String> {
    match filter {
        "url" => crate::url::extract_urls(text),
        "path" => parse_paths(text),
        "word" => parse_words(text),
        "line" => parse_lines(text),
        _ => {
            let mut seen = HashSet::new();
            let mut out = Vec::new();
            for t in crate::url::extract_urls(text)
                .into_iter()
                .chain(parse_paths(text))
                .chain(parse_words(text))
            {
                if seen.insert(t.clone()) {
                    out.push(t);
                }
            }
            out
        }
    }
}

pub fn run(pane: Option<&str>, source: Option<&str>) -> Result<()> {
    match source {
        None => {
            let pane = pane.ok_or_else(|| anyhow::anyhow!("grab needs --pane"))?;
            launch(pane)
        }
        Some(file) => pick_loop(file, pane.unwrap_or("")),
    }
}

fn launch(pane: &str) -> Result<()> {
    let text = crate::tmux::run(&["capture-pane", "-p", "-J", "-t", pane, "-S", "-100"])
        .unwrap_or_default();
    if tokens_for("all", &text).is_empty() {
        crate::tmux::run_ok(&["display-message", "anka: nothing to grab"]);
        return Ok(());
    }
    let tmp = std::env::temp_dir().join(format!("anka-grab-{}.txt", pane.trim_start_matches('%')));
    std::fs::write(&tmp, &text)?;
    let exe = std::env::current_exe()?;
    let cmd = format!("{} grab {} --pane {}", exe.display(), tmp.display(), pane);
    crate::tmux::run_ok(&["display-popup", "-w", "70%", "-h", "70%", "-E", &cmd]);
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

fn pick_loop(file: &str, pane: &str) -> Result<()> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let mut fi = 0usize;
    loop {
        let items = tokens_for(FILTERS[fi], &text);
        if items.is_empty() {
            return Ok(());
        }
        let title = format!("grab · {} · Tab filter · ^v paste · ^o open", FILTERS[fi]);
        match picker::pick_ex(&items, &title)? {
            Hit::Enter(i) => {
                crate::clip::copy(items[i].as_bytes(), false);
                return Ok(());
            }
            Hit::Ctrl('v', i) => {
                crate::tmux::run_ok(&["send-keys", "-t", pane, "-l", &items[i]]);
                return Ok(());
            }
            Hit::Ctrl('o', i) => {
                crate::url::open(&items[i]);
                return Ok(());
            }
            Hit::Ctrl(_, _) => {}
            Hit::Tab => {
                for _ in 0..FILTERS.len() {
                    fi = (fi + 1) % FILTERS.len();
                    if !tokens_for(FILTERS[fi], &text).is_empty() {
                        break;
                    }
                }
            }
            Hit::Cancel => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_trims_and_dedupes() {
        assert_eq!(parse_lines("  a \n\n a \n b "), vec!["a", "b"]);
    }

    #[test]
    fn words_min_len_and_trim() {
        assert_eq!(parse_words("(foo) a bar, foo"), vec!["foo", "bar"]);
    }

    #[test]
    fn paths_detected_not_plain_words() {
        assert_eq!(
            parse_paths("see /etc/hosts and ./src/main.rs not justaword or github.com/x"),
            vec!["/etc/hosts", "./src/main.rs"]
        );
    }

    #[test]
    fn all_unions_url_path_word() {
        let t = tokens_for("all", "go https://x.com/a /tmp/f.txt hello");
        assert_eq!(t[0], "https://x.com/a");
        assert!(t.contains(&"/tmp/f.txt".to_string()));
        assert!(t.contains(&"hello".to_string()));
    }
}

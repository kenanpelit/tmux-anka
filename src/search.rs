//! `anka search` — fuzzy-search the scrollback, then jump via tmux's own
//! copy-mode search. Using `search-backward` (instead of goto-line arithmetic)
//! means tmux lands on the exact match AND highlights the term — no fragile
//! line-number math. We land on the picked line, then refine onto the typed
//! word so it's the highlighted pattern.

use std::collections::HashSet;

use anyhow::Result;

use crate::picker;
use crate::tmux;

/// Escape a string for tmux's copy-mode (extended regex) search.
pub fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if matches!(
            c,
            '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Non-empty scrollback lines, trimmed, newest-first, de-duplicated.
pub fn pick_lines(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    text.lines()
        .rev()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .filter(|l| seen.insert(l.clone()))
        .collect()
}

pub fn run(pane: Option<&str>, source: Option<&str>) -> Result<()> {
    match source {
        None => {
            let pane = pane.ok_or_else(|| anyhow::anyhow!("search needs --pane"))?;
            launch(pane)
        }
        Some(file) => jump_from(file, pane.unwrap_or("")),
    }
}

fn launch(pane: &str) -> Result<()> {
    let text = tmux::run(&["capture-pane", "-p", "-t", pane, "-S", "-"]).unwrap_or_default();
    if text.lines().all(|l| l.trim().is_empty()) {
        tmux::run_ok(&["display-message", "anka: no scrollback"]);
        return Ok(());
    }
    let tmp = std::env::temp_dir().join(format!("anka-search-{}.txt", pane.trim_start_matches('%')));
    std::fs::write(&tmp, &text)?;
    let exe = std::env::current_exe()?;
    let cmd = format!("{} search {} --pane {}", exe.display(), tmp.display(), pane);
    tmux::run_ok(&["display-popup", "-w", "80%", "-h", "70%", "-E", &cmd]);
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

fn jump_from(file: &str, pane: &str) -> Result<()> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let lines = pick_lines(&text);
    if lines.is_empty() {
        return Ok(());
    }
    let Some((sel, query)) = picker::pick_q(&lines, "search")? else {
        return Ok(());
    };
    let line = lines[sel].as_str();

    tmux::run_ok(&["copy-mode", "-t", pane]);
    // Land on the exact picked line.
    tmux::run_ok(&["send-keys", "-X", "-t", pane, "search-backward", &regex_escape(line)]);
    // Refine onto the typed word so it becomes the highlighted pattern.
    if !query.is_empty() && line.contains(&query) {
        tmux::run_ok(&["send-keys", "-X", "-t", pane, "search-forward", &regex_escape(&query)]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_regex_metachars() {
        assert_eq!(regex_escape("a.b*c"), "a\\.b\\*c");
        assert_eq!(regex_escape("err[0]"), "err\\[0\\]");
        assert_eq!(regex_escape("plain"), "plain");
    }

    #[test]
    fn lines_newest_first_deduped_nonempty() {
        let txt = "a\n\n b \nb\na\n";
        // reversed: "a","b"," b"(->"b" dup),"","a"(dup) → ["a","b"]
        assert_eq!(pick_lines(txt), vec!["a".to_string(), "b".to_string()]);
    }
}

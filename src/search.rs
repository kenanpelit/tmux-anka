//! `anka search` — fuzzy-search the scrollback, jump to the line in copy-mode
//! (replaces tmux-fuzzback). Line precision; column precision is out of scope.
//! Two-step launch like `anka url`: capture the scrollback, reopen in a popup.

use anyhow::Result;

use crate::picker;

/// copy-mode `goto-line` target for a line at `top_index` (0 = oldest captured
/// line). `capture-pane -S -` and copy-mode share the same numbering, so the
/// index *is* the target; clamped to the last line.
pub fn goto_line_target(total: usize, top_index: usize) -> usize {
    top_index.min(total.saturating_sub(1))
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
    let text =
        crate::tmux::run(&["capture-pane", "-p", "-J", "-t", pane, "-S", "-"]).unwrap_or_default();
    if text.lines().all(|l| l.trim().is_empty()) {
        crate::tmux::run_ok(&["display-message", "anka: no scrollback"]);
        return Ok(());
    }
    let tmp = std::env::temp_dir().join(format!("anka-search-{}.txt", pane.trim_start_matches('%')));
    std::fs::write(&tmp, &text)?;
    let exe = std::env::current_exe()?;
    let cmd = format!("{} search {} --pane {}", exe.display(), tmp.display(), pane);
    crate::tmux::run_ok(&["display-popup", "-w", "80%", "-h", "70%", "-E", &cmd]);
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

fn jump_from(file: &str, pane: &str) -> Result<()> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let mut items: Vec<String> = Vec::new();
    let mut idxs: Vec<usize> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        if !t.is_empty() {
            items.push(format!("{:>5}  {t}", i + 1));
            idxs.push(i);
        }
    }
    // newest first
    items.reverse();
    idxs.reverse();
    if items.is_empty() {
        return Ok(());
    }
    if let Some(sel) = picker::pick(&items, "search")? {
        let target = goto_line_target(total, idxs[sel]);
        crate::tmux::run_ok(&["copy-mode", "-t", pane]);
        crate::tmux::run_ok(&["send-keys", "-X", "-t", pane, "goto-line", &target.to_string()]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_is_index_clamped() {
        assert_eq!(goto_line_target(100, 42), 42);
        assert_eq!(goto_line_target(10, 99), 9);
        assert_eq!(goto_line_target(0, 0), 0);
    }
}

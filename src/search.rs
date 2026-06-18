//! `anka search` — fuzzy-search the scrollback, jump to the line in copy-mode
//! (port of tmux-fuzzback's jump). Numbering is relative to the cursor at start
//! (head = at/above cursor, tail = below); the jump uses `goto-line` with the
//! same max-jump correction + centering as fuzzback, then positions the cursor
//! at the query column.

use anyhow::Result;

use crate::picker;

#[derive(Debug, PartialEq)]
pub struct SearchItem {
    /// 1 = at/above the start cursor (head), -1 = below (tail).
    pub direction: i32,
    /// Distance from the cursor (head: 0 = cursor line, up) / from the top of
    /// the tail (tail: 0 = just below cursor).
    pub line_number: usize,
    pub text: String,
}

#[derive(Debug, PartialEq)]
pub enum Step {
    Goto(usize),
    Up(usize),
    Down(usize),
}

/// Lines of the head (at/above the start cursor), i.e. fuzzback's `head_n`.
pub fn head_len(total: usize, cursor_y: usize, pane_height: usize) -> usize {
    let pos_rev = pane_height.saturating_sub(cursor_y);
    (total + 1).saturating_sub(pos_rev).min(total)
}

/// Build the picker items with fuzzback's cursor-relative numbering.
pub fn build_items(lines: &[String], cursor_y: usize, pane_height: usize) -> Vec<SearchItem> {
    let total = lines.len();
    let pos_rev = pane_height.saturating_sub(cursor_y);
    let head_n = head_len(total, cursor_y, pane_height);
    let tail_n = pos_rev.saturating_sub(1).min(total);

    let mut items = Vec::new();
    // Head: reversed (cursor line first, going up).
    for p in 0..head_n {
        items.push(SearchItem {
            direction: 1,
            line_number: p,
            text: lines[head_n - 1 - p].clone(),
        });
    }
    // Tail: numbered from the top of the tail (just below the cursor) going down.
    for p in 0..tail_n {
        items.push(SearchItem {
            direction: -1,
            line_number: p,
            text: lines[total - tail_n + p].clone(),
        });
    }
    items
}

/// The copy-mode steps to reach a selected line (port of fuzzback `goto_line`).
pub fn plan_jump(direction: i32, line_number: usize, max_lines: usize, pane_height: usize) -> Vec<Step> {
    let mut s = Vec::new();
    if direction >= 0 {
        let max_jump = max_lines.saturating_sub(pane_height);
        if line_number <= max_jump {
            s.push(Step::Goto(line_number));
            // centre the result, up to half a pane height of padding
            let pad = line_number.min(pane_height / 2);
            if pad > 0 {
                s.push(Step::Down(pad));
                s.push(Step::Up(pad));
            }
        } else {
            s.push(Step::Goto(max_jump));
            s.push(Step::Up(max_lines));
            s.push(Step::Down(max_lines.saturating_sub(line_number + 1)));
        }
    } else {
        s.push(Step::Goto(0));
        s.push(Step::Down(line_number + 1));
    }
    s
}

/// 0-based character column of `query` within `line` (0 if absent/empty).
pub fn query_column(line: &str, query: &str) -> usize {
    if query.is_empty() {
        return 0;
    }
    line.find(query)
        .map(|b| line[..b].chars().count())
        .unwrap_or(0)
}

pub fn run(
    pane: Option<&str>,
    source: Option<&str>,
    cursor_y: Option<usize>,
    pane_height: Option<usize>,
) -> Result<()> {
    match source {
        None => {
            let pane = pane.ok_or_else(|| anyhow::anyhow!("search needs --pane"))?;
            launch(pane)
        }
        Some(file) => jump_from(file, pane.unwrap_or(""), cursor_y.unwrap_or(0), pane_height.unwrap_or(0)),
    }
}

fn display_usize(pane: &str, fmt: &str) -> usize {
    crate::tmux::run(&["display-message", "-p", "-t", pane, fmt])
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn launch(pane: &str) -> Result<()> {
    // Capture state BEFORE entering copy-mode (cursor at the prompt).
    let cursor_y = display_usize(pane, "#{cursor_y}");
    let pane_height = display_usize(pane, "#{pane_height}");
    let text =
        crate::tmux::run(&["capture-pane", "-p", "-t", pane, "-S", "-"]).unwrap_or_default();
    if text.lines().all(|l| l.trim().is_empty()) {
        crate::tmux::run_ok(&["display-message", "anka: no scrollback"]);
        return Ok(());
    }
    let tmp = std::env::temp_dir().join(format!("anka-search-{}.txt", pane.trim_start_matches('%')));
    std::fs::write(&tmp, &text)?;
    let exe = std::env::current_exe()?;
    let cmd = format!(
        "{} search {} --pane {} --cursor-y {} --pane-height {}",
        exe.display(),
        tmp.display(),
        pane,
        cursor_y,
        pane_height
    );
    crate::tmux::run_ok(&["display-popup", "-w", "80%", "-h", "70%", "-E", &cmd]);
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

fn send_x(pane: &str, args: &[&str]) {
    let mut v = vec!["send-keys", "-X", "-t", pane];
    v.extend_from_slice(args);
    crate::tmux::run_ok(&v);
}

fn jump_from(file: &str, pane: &str, cursor_y: usize, pane_height: usize) -> Result<()> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let lines: Vec<String> = text.lines().map(String::from).collect();
    let items = build_items(&lines, cursor_y, pane_height);
    if items.is_empty() {
        return Ok(());
    }
    let labels: Vec<String> = items.iter().map(|it| it.text.trim().to_string()).collect();
    let Some((sel, query)) = picker::pick_q(&labels, "search")? else {
        return Ok(());
    };
    let it = &items[sel];
    let max_lines = head_len(lines.len(), cursor_y, pane_height);
    let steps = plan_jump(it.direction, it.line_number, max_lines, pane_height);
    let column = query_column(&it.text, &query);

    crate::tmux::run_ok(&["copy-mode", "-t", pane]);
    for step in steps {
        match step {
            Step::Goto(n) => send_x(pane, &["goto-line", &n.to_string()]),
            Step::Up(n) if n > 0 => send_x(pane, &["-N", &n.to_string(), "cursor-up"]),
            Step::Down(n) if n > 0 => send_x(pane, &["-N", &n.to_string(), "cursor-down"]),
            _ => {}
        }
    }
    send_x(pane, &["start-of-line"]);
    if column > 0 {
        send_x(pane, &["-N", &column.to_string(), "cursor-right"]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("L{i}")).collect()
    }

    #[test]
    fn head_tail_numbering() {
        // 5 lines, cursor_y=3, pane_height=5 → pos_rev=2, head_n=4, tail_n=1
        let items = build_items(&lines(5), 3, 5);
        assert_eq!(items[0], SearchItem { direction: 1, line_number: 0, text: "L4".into() });
        assert_eq!(items[3], SearchItem { direction: 1, line_number: 3, text: "L1".into() });
        assert_eq!(items[4], SearchItem { direction: -1, line_number: 0, text: "L5".into() });
        assert_eq!(head_len(5, 3, 5), 4);
    }

    #[test]
    fn jump_reachable_centers() {
        // line_number 5 ≤ max_jump(80) → Goto(5) + center pad min(5,10)=5
        assert_eq!(
            plan_jump(1, 5, 100, 20),
            vec![Step::Goto(5), Step::Down(5), Step::Up(5)]
        );
    }

    #[test]
    fn jump_beyond_max_corrects() {
        // line_number 90 > max_jump(80) → Goto(80), Up(100), Down(100-91=9)
        assert_eq!(
            plan_jump(1, 90, 100, 20),
            vec![Step::Goto(80), Step::Up(100), Step::Down(9)]
        );
    }

    #[test]
    fn jump_tail_goes_down_from_top() {
        assert_eq!(plan_jump(-1, 3, 0, 20), vec![Step::Goto(0), Step::Down(4)]);
    }

    #[test]
    fn column_is_char_index() {
        assert_eq!(query_column("foo bar baz", "bar"), 4);
        assert_eq!(query_column("héllo wörld", "wörld"), 6);
        assert_eq!(query_column("nope", "zzz"), 0);
        assert_eq!(query_column("anything", ""), 0);
    }
}

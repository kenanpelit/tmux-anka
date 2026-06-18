//! Pure switcher model + fuzzy matcher (no tmux, no I/O).

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Live(String),
    Snapshot(String),
    Window { session: String, index: u32, name: String },
    Zoxide(PathBuf),
}

pub fn item_label(i: &Item) -> String {
    match i {
        Item::Live(n) => format!("{n}  (live)"),
        Item::Snapshot(n) => format!("{n}  (snapshot)"),
        Item::Window { session, index, name } => format!("{session}:{index} {name}"),
        Item::Zoxide(p) => p.display().to_string(),
    }
}

/// The display name without the trailing live/snapshot tag (the switcher draws
/// that tag itself, coloured).
pub fn item_name(i: &Item) -> String {
    match i {
        Item::Live(n) | Item::Snapshot(n) => n.clone(),
        Item::Window { session, index, name } => format!("{session}:{index} {name}"),
        Item::Zoxide(p) => p.display().to_string(),
    }
}

/// The string a query is fuzzy-matched against.
pub fn item_key(i: &Item) -> String {
    match i {
        Item::Live(n) | Item::Snapshot(n) => n.clone(),
        Item::Window { session, name, .. } => format!("{session} {name}"),
        Item::Zoxide(p) => p.display().to_string(),
    }
}

/// Live sessions first, then snapshot-only sessions (a live session hides its
/// snapshot twin).
pub fn build_session_items(live: &[String], snapshot: &[String]) -> Vec<Item> {
    let mut items: Vec<Item> = live.iter().cloned().map(Item::Live).collect();
    for s in snapshot {
        if !live.iter().any(|l| l == s) {
            items.push(Item::Snapshot(s.clone()));
        }
    }
    items
}

/// Case-insensitive subsequence match. Returns `None` if `query` is not a
/// subsequence of `hay`; otherwise a score where contiguous and earlier matches
/// score higher (prefix matches rank best). An empty query matches everything.
pub fn fuzzy_score(query: &str, hay: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let h: Vec<char> = hay.to_lowercase().chars().collect();
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let mut qi = 0;
    let mut score = 0i32;
    let mut last_match: Option<usize> = None;
    for (hi, hc) in h.iter().enumerate() {
        if qi < q.len() && *hc == q[qi] {
            score += 10;
            if hi == 0 {
                score += 20; // prefix bonus
            }
            if let Some(prev) = last_match {
                if hi == prev + 1 {
                    score += 5; // contiguity bonus
                }
            }
            score -= hi as i32; // earlier is better
            last_match = Some(hi);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score)
}

/// Char indices of `hay` matched by `query` (greedy subsequence, case-insensitive),
/// or `None` if `query` isn't a subsequence. Indices align with `hay.chars()` so
/// callers can highlight the matched characters.
pub fn fuzzy_positions(query: &str, hay: &str) -> Option<Vec<usize>> {
    if query.is_empty() {
        return Some(Vec::new());
    }
    let q: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let mut qi = 0;
    let mut pos = Vec::new();
    for (hi, hc) in hay.chars().enumerate() {
        if qi >= q.len() {
            break;
        }
        if hc.to_lowercase().next() == Some(q[qi]) {
            pos.push(hi);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(pos)
}

#[cfg(test)]
mod fuzzy_tests {
    use super::*;

    #[test]
    fn positions_subsequence() {
        assert_eq!(fuzzy_positions("ac", "abc"), Some(vec![0, 2]));
        assert_eq!(fuzzy_positions("AB", "xaybz"), Some(vec![1, 3]));
        assert_eq!(fuzzy_positions("", "abc"), Some(vec![]));
        assert_eq!(fuzzy_positions("xyz", "abc"), None);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Sessions,
    Windows,
    Zoxide,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Tab,
    Backspace,
    Char(char),
    Digit(usize), // 1-9: jump straight to (and activate) that row
    Rename,
    Delete,
    Ctrl(char),
    Cancel,
}

/// Effects that keep the switcher open and reload its items afterwards.
#[derive(Debug)]
pub enum Stay {
    Kill(Item),
    Rename { item: Item, to: String },
}

/// Effects that close the switcher (popup).
#[derive(Debug)]
pub enum Exit {
    Activate(Item),
    NewSession(String),
    Cancel,
}

#[derive(Debug)]
pub enum Step {
    Redraw,
    PreviewChanged,
    Stay(Stay),
    Exit(Exit),
}

enum Prompt {
    Rename(String),
}

pub struct State {
    mode: Mode,
    items: Vec<Item>,
    filtered: Vec<usize>,
    query: String,
    cursor: usize,
    prompt: Option<Prompt>,
}

impl State {
    pub fn new(items: Vec<Item>, mode: Mode) -> Self {
        let mut s = State {
            mode,
            items,
            filtered: vec![],
            query: String::new(),
            cursor: 0,
            prompt: None,
        };
        s.refilter();
        s
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// Replace the item set (used when the mode changes), resetting the cursor.
    pub fn set_items(&mut self, items: Vec<Item>) {
        self.items = items;
        self.cursor = 0;
        self.refilter();
    }

    pub fn visible(&self) -> Vec<&Item> {
        self.filtered.iter().map(|&i| &self.items[i]).collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selected(&self) -> Option<&Item> {
        self.filtered.get(self.cursor).map(|&i| &self.items[i])
    }

    /// `(label, buffer)` while a Rename name is being typed.
    pub fn prompt(&self) -> Option<(&'static str, &str)> {
        match &self.prompt {
            Some(Prompt::Rename(b)) => Some(("Rename to:", b)),
            None => None,
        }
    }

    fn refilter(&mut self) {
        let mut scored: Vec<(usize, i32)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| fuzzy_score(&self.query, &item_key(it)).map(|sc| (i, sc)))
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        if self.cursor >= self.filtered.len() {
            self.cursor = self.filtered.len().saturating_sub(1);
        }
    }

    pub fn apply(&mut self, key: Key) -> Step {
        if self.prompt.is_some() {
            return self.apply_prompt(key);
        }
        match key {
            Key::Char(c) => {
                self.query.push(c);
                self.refilter();
                Step::Redraw
            }
            Key::Backspace => {
                self.query.pop();
                self.refilter();
                Step::Redraw
            }
            Key::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                Step::PreviewChanged
            }
            Key::Down => {
                if self.cursor + 1 < self.filtered.len() {
                    self.cursor += 1;
                }
                Step::PreviewChanged
            }
            Key::Tab => {
                self.mode = match self.mode {
                    Mode::Sessions => Mode::Windows,
                    Mode::Windows => Mode::Zoxide,
                    Mode::Zoxide => Mode::Sessions,
                };
                Step::Redraw
            }
            Key::Enter => match self.selected().cloned() {
                Some(it) => Step::Exit(Exit::Activate(it)),
                // Nothing matches the query → create a session named after it.
                None if !self.query.is_empty() => Step::Exit(Exit::NewSession(self.query.clone())),
                None => Step::Exit(Exit::Cancel),
            },
            // Digit hotkey: jump straight to that row and activate it.
            Key::Digit(n) => match self.filtered.get(n.wrapping_sub(1)) {
                Some(&i) => Step::Exit(Exit::Activate(self.items[i].clone())),
                None => Step::Redraw,
            },
            Key::Rename => match self.selected() {
                Some(Item::Live(n)) | Some(Item::Snapshot(n)) => {
                    self.prompt = Some(Prompt::Rename(n.clone()));
                    Step::Redraw
                }
                _ => Step::Redraw,
            },
            Key::Delete => match self.selected().cloned() {
                Some(it @ Item::Live(_)) => Step::Stay(Stay::Kill(it)),
                _ => Step::Redraw,
            },
            Key::Ctrl(_) => Step::Redraw,
            Key::Cancel => Step::Exit(Exit::Cancel),
        }
    }

    fn apply_prompt(&mut self, key: Key) -> Step {
        let Prompt::Rename(buf) = self.prompt.as_mut().unwrap();
        match key {
            Key::Char(c) => {
                buf.push(c);
                Step::Redraw
            }
            Key::Digit(n) => {
                // In a prompt, digits are literal text, not a jump.
                buf.push(char::from_digit(n as u32, 10).unwrap_or('0'));
                Step::Redraw
            }
            Key::Backspace => {
                buf.pop();
                Step::Redraw
            }
            Key::Cancel => {
                self.prompt = None;
                Step::Redraw
            }
            Key::Enter => {
                let Prompt::Rename(to) = self.prompt.take().unwrap();
                match self.selected().cloned() {
                    Some(item) => Step::Stay(Stay::Rename { item, to }),
                    None => Step::Redraw,
                }
            }
            _ => Step::Redraw,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_merges_and_dedupes() {
        let items = build_session_items(&["KENP".into(), "Tor".into()], &["KENP".into(), "old".into()]);
        let labels: Vec<String> = items.iter().map(item_label).collect();
        assert!(labels.iter().any(|l| l.contains("KENP") && l.contains("live")));
        assert!(labels.iter().any(|l| l.contains("old") && l.contains("snapshot")));
        assert_eq!(labels.iter().filter(|l| l.contains("KENP")).count(), 1);
    }

    #[test]
    fn fuzzy_matches_subsequence_and_ranks_prefix_higher() {
        assert!(fuzzy_score("dv", "dev").is_some());
        assert!(fuzzy_score("xyz", "dev").is_none());
        assert!(fuzzy_score("dev", "dev").unwrap() > fuzzy_score("dev", "my-dev-box").unwrap());
    }

    #[test]
    fn empty_query_matches_everything() {
        assert!(fuzzy_score("", "anything").is_some());
    }
}

#[cfg(test)]
mod apply_tests {
    use super::*;

    fn st() -> State {
        State::new(
            vec![
                Item::Live("KENP".into()),
                Item::Live("dev".into()),
                Item::Snapshot("old".into()),
            ],
            Mode::Sessions,
        )
    }

    #[test]
    fn typing_filters_and_resets_cursor() {
        let mut s = st();
        s.apply(Key::Char('d')); // matches "dev" and "old" (o-l-d)
        assert!(matches!(s.apply(Key::Char('e')), Step::Redraw)); // "de" → only "dev"
        assert_eq!(s.visible().len(), 1);
        assert_eq!(item_key(s.selected().unwrap()), "dev");
    }

    #[test]
    fn down_moves_selection_and_requests_preview() {
        let mut s = st();
        assert!(matches!(s.apply(Key::Down), Step::PreviewChanged));
        assert_eq!(item_key(s.selected().unwrap()), "dev");
    }

    #[test]
    fn enter_activates_selected() {
        let mut s = st();
        match s.apply(Key::Enter) {
            Step::Exit(Exit::Activate(Item::Live(n))) => assert_eq!(n, "KENP"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn cancel_exits() {
        let mut s = st();
        assert!(matches!(s.apply(Key::Cancel), Step::Exit(Exit::Cancel)));
    }

    #[test]
    fn tab_cycles_mode() {
        let mut s = st();
        s.apply(Key::Tab);
        assert_eq!(s.mode(), Mode::Windows);
        s.apply(Key::Tab);
        assert_eq!(s.mode(), Mode::Zoxide);
        s.apply(Key::Tab);
        assert_eq!(s.mode(), Mode::Sessions);
    }

    #[test]
    fn enter_on_no_match_creates_named_session() {
        let mut s = st();
        for c in "wx".chars() {
            s.apply(Key::Char(c)); // "wx" matches nothing
        }
        assert!(s.visible().is_empty());
        match s.apply(Key::Enter) {
            Step::Exit(Exit::NewSession(name)) => assert_eq!(name, "wx"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn digit_jumps_to_and_activates_row() {
        let mut s = st(); // [KENP, dev, old]
        match s.apply(Key::Digit(2)) {
            Step::Exit(Exit::Activate(Item::Live(n))) => assert_eq!(n, "dev"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn out_of_range_digit_is_ignored() {
        let mut s = st();
        assert!(matches!(s.apply(Key::Digit(9)), Step::Redraw));
    }

    #[test]
    fn digit_in_rename_prompt_is_literal() {
        let mut s = st();
        s.apply(Key::Rename); // KENP
        for _ in 0..4 {
            s.apply(Key::Backspace);
        }
        s.apply(Key::Char('w'));
        s.apply(Key::Digit(2));
        assert_eq!(s.prompt().map(|(_, b)| b), Some("w2"));
    }

    #[test]
    fn rename_prompt_prefills_name_and_yields_stay() {
        let mut s = st();
        s.apply(Key::Rename); // selected = KENP
        assert_eq!(s.prompt().map(|(_, b)| b), Some("KENP"));
        // clear + retype
        for _ in 0..4 {
            s.apply(Key::Backspace);
        }
        for c in "knp".chars() {
            s.apply(Key::Char(c));
        }
        match s.apply(Key::Enter) {
            Step::Stay(Stay::Rename { item: Item::Live(n), to }) => {
                assert_eq!(n, "KENP");
                assert_eq!(to, "knp");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn delete_on_live_is_stay_kill() {
        let mut s = st();
        match s.apply(Key::Delete) {
            Step::Stay(Stay::Kill(Item::Live(n))) => assert_eq!(n, "KENP"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn prompt_cancel_returns_to_list() {
        let mut s = st();
        s.apply(Key::Rename);
        assert!(s.prompt().is_some());
        assert!(matches!(s.apply(Key::Cancel), Step::Redraw));
        assert!(s.prompt().is_none());
    }
}

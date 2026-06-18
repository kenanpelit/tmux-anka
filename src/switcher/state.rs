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

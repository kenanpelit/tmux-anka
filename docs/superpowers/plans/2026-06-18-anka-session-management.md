# anka Native Session Management — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring `tmux-sessionist` (quick session actions) and `tmux-sessionx` (a fuzzy session switcher with preview) into anka natively, then remove those plugins.

**Architecture:** Quick actions live in `src/session.rs` (thin tmux wrappers). The switcher is split into a pure state machine (`src/switcher/state.rs`) + a thin I/O shell (`src/switcher/term.rs`, `src/switcher/mod.rs`); all logic is unit-tested without a terminal, and the loop falls back to a numbered menu when stdin/stdout is not a tty (also what tests drive).

**Tech Stack:** Rust (clap, serde, anyhow), tmux CLI, `stty` for raw mode, optional `zoxide`.

## Global Constraints

- Zero runtime dependencies; no new crates (no TUI/fzf/zoxide crate). `zoxide` is shelled out to, optional.
- Linux-only (`/proc`, `stty`).
- Every config option read via `tmux show-options -gqv`, with a default; option names are `@anka-*`.
- Restore the terminal (`stty sane`) on normal exit; never rely on Drop for correctness (`panic = abort`).
- Keep the binary tiny; follow the existing hand-formatted style (no global `cargo fmt`).
- Run tests with `cargo test --offline`; lint with `cargo clippy --offline --all-targets` (must be warning-clean).
- Version bumps land in both `Cargo.toml` and `Cargo.lock`.

---

### Task 1: `anka session` quick actions (sessionist replacement)

**Files:**
- Create: `src/session.rs`
- Modify: `src/cli.rs` (add `Session` subcommand), `src/main.rs` (dispatch)
- Test: `tests/session_integration.rs`, `tests/common/mod.rs` (already has `Server`)

**Interfaces:**
- Produces: `session::run(action: SessionAction) -> Result<()>`; `enum SessionAction { New(String), Kill, Promote(String), Switch(String), Last, Rename(String) }`.
- Consumes: `tmux::run`, `tmux::run_ok`, `tmux::server_running` (existing).

- [ ] **Step 1: CLI surface.** In `src/cli.rs`, add to `enum Cmd`:

```rust
    /// Session management actions (sessionist-style)
    Session {
        #[command(subcommand)]
        action: SessionCmd,
    },
```
and a new enum:
```rust
#[derive(Subcommand)]
pub enum SessionCmd {
    /// Create or switch to a named session
    New { name: String },
    /// Kill the current session, switching away first
    Kill,
    /// Move the current pane into a new session
    Promote { name: String },
    /// Switch to a session by name
    Switch { name: String },
    /// Switch to the last session
    Last,
    /// Rename the current session
    Rename { name: String },
}
```

- [ ] **Step 2: Dispatch.** In `src/main.rs`, add `mod session;` and a match arm:
```rust
        Cmd::Session { action } => session::run(action),
```
Map `SessionCmd` → `session::run`. Use `cli::SessionCmd` directly as the param.

- [ ] **Step 3: Write failing integration tests** in `tests/session_integration.rs`:

```rust
//! Integration tests for `anka session …`, driving a throwaway server.
mod common;
use common::*;

#[test]
fn session_new_creates_and_switches() {
    if !has_tmux() { eprintln!("skip: no tmux"); return; }
    let s = Server::start("sess-new");
    assert!(s.anka(&["session", "new", "work"]).status.success());
    assert!(sessions(&s.socket).contains(&"work".to_string()));
}

#[test]
fn session_rename_renames_current() {
    if !has_tmux() { eprintln!("skip: no tmux"); return; }
    let s = Server::start("sess-rn");
    s.tmux(&["new-session", "-d", "-s", "old", "-x", "200", "-y", "50"]);
    // attach context: anka acts on the client's session; drive via -t by switching
    s.tmux(&["switch-client", "-t", "old"]);
    assert!(s.anka(&["session", "rename", "newname"]).status.success());
    let names = sessions(&s.socket);
    assert!(names.contains(&"newname".to_string()) && !names.contains(&"old".to_string()), "{names:?}");
}

#[test]
fn session_kill_refuses_last_session() {
    if !has_tmux() { eprintln!("skip: no tmux"); return; }
    let s = Server::start("sess-kill1");
    // only "scratch" exists; killing it would drop the server → must refuse
    let out = s.anka(&["session", "kill"]);
    assert!(!out.status.success(), "kill should refuse the last session");
    assert!(sessions(&s.socket).contains(&"scratch".to_string()));
}

#[test]
fn session_kill_switches_then_kills() {
    if !has_tmux() { eprintln!("skip: no tmux"); return; }
    let s = Server::start("sess-kill2");
    s.tmux(&["new-session", "-d", "-s", "victim", "-x", "200", "-y", "50"]);
    s.tmux(&["switch-client", "-t", "victim"]);
    assert!(s.anka(&["session", "kill"]).status.success());
    assert!(!sessions(&s.socket).contains(&"victim".to_string()));
}

#[test]
fn session_promote_moves_pane_to_new_session() {
    if !has_tmux() { eprintln!("skip: no tmux"); return; }
    let s = Server::start("sess-prom");
    s.tmux(&["new-session", "-d", "-s", "src", "-x", "200", "-y", "50"]);
    s.tmux(&["split-window", "-t", "src", "-x", "200", "-y", "50"]);
    s.tmux(&["switch-client", "-t", "src"]);
    assert!(s.anka(&["session", "promote", "promoted"]).status.success());
    assert!(sessions(&s.socket).contains(&"promoted".to_string()));
}
```
Note: `anka` reads the active client session via `#{client_session}`; the `Server` sets `TMUX`, so `switch-client` selects which session anka treats as current. Add a `current_session()` helper in `session.rs` using `display-message -p '#{client_session}'`, falling back to the only session.

- [ ] **Step 4: Run tests, verify they fail.** `cargo test --offline --test session_integration` → FAIL (module missing).

- [ ] **Step 5: Implement `src/session.rs`:**

```rust
//! Sessionist-style quick session actions.

use anyhow::{bail, Result};

use crate::cli::SessionCmd;
use crate::tmux;

pub fn run(action: SessionCmd) -> Result<()> {
    if !tmux::server_running() {
        bail!("no tmux server running");
    }
    match action {
        SessionCmd::New { name } => new(&name),
        SessionCmd::Kill => kill(),
        SessionCmd::Promote { name } => promote(&name),
        SessionCmd::Switch { name } => switch(&name),
        SessionCmd::Last => last(),
        SessionCmd::Rename { name } => rename(&name),
    }
}

fn current_session() -> Result<String> {
    let s = tmux::run(&["display-message", "-p", "#{client_session}"]).unwrap_or_default();
    if !s.is_empty() {
        return Ok(s);
    }
    // No attached client (e.g. scripted): fall back to the only/first session.
    let list = tmux::run(&["list-sessions", "-F", "#{session_name}"]).unwrap_or_default();
    list.lines().next().map(String::from).ok_or_else(|| anyhow::anyhow!("no sessions"))
}

fn sessions() -> Vec<String> {
    tmux::run(&["list-sessions", "-F", "#{session_name}"])
        .map(|s| s.lines().map(String::from).collect())
        .unwrap_or_default()
}

fn new(name: &str) -> Result<()> {
    if sessions().iter().any(|s| s == name) {
        tmux::run_ok(&["switch-client", "-t", name]);
        println!("switched to existing session '{name}'");
        return Ok(());
    }
    let cwd = tmux::run(&["display-message", "-p", "#{pane_current_path}"]).unwrap_or_default();
    let mut args = vec!["new-session", "-d", "-s", name];
    if !cwd.is_empty() {
        args.push("-c");
        args.push(&cwd);
    }
    tmux::run(&args)?;
    tmux::run_ok(&["switch-client", "-t", name]);
    println!("created session '{name}'");
    Ok(())
}

fn kill() -> Result<()> {
    let cur = current_session()?;
    let others: Vec<String> = sessions().into_iter().filter(|s| *s != cur).collect();
    let Some(target) = others.first() else {
        bail!("refusing to kill the last session '{cur}' (would stop the server)");
    };
    tmux::run_ok(&["switch-client", "-t", target]);
    tmux::run(&["kill-session", "-t", &cur])?;
    println!("killed session '{cur}', switched to '{target}'");
    Ok(())
}

fn promote(name: &str) -> Result<()> {
    if sessions().iter().any(|s| s == name) {
        bail!("session '{name}' already exists");
    }
    let pane = tmux::run(&["display-message", "-p", "#{pane_id}"])?;
    let cwd = tmux::run(&["display-message", "-p", "#{pane_current_path}"]).unwrap_or_default();
    // Create a placeholder session, move our pane in beside it, drop the placeholder.
    let mut args = vec!["new-session", "-d", "-s", name];
    if !cwd.is_empty() {
        args.push("-c");
        args.push(&cwd);
    }
    tmux::run(&args)?;
    let placeholder = tmux::run(&["display-message", "-p", "-t", name, "#{pane_id}"])?;
    tmux::run(&["move-pane", "-s", &pane, "-t", &format!("{name}.")])?;
    tmux::run_ok(&["kill-pane", "-t", &placeholder]);
    tmux::run_ok(&["select-layout", "-t", name, "tiled"]);
    tmux::run_ok(&["switch-client", "-t", name]);
    println!("promoted pane into new session '{name}'");
    Ok(())
}

fn switch(name: &str) -> Result<()> {
    if !sessions().iter().any(|s| s == name) {
        bail!("session '{name}' not found");
    }
    tmux::run(&["switch-client", "-t", name])?;
    Ok(())
}

fn last() -> Result<()> {
    tmux::run_ok(&["switch-client", "-l"]);
    Ok(())
}

fn rename(name: &str) -> Result<()> {
    tmux::run(&["rename-session", name])?;
    println!("renamed session to '{name}'");
    Ok(())
}
```

- [ ] **Step 6: Run tests, verify pass.** `cargo test --offline --test session_integration` → all PASS. Then `cargo clippy --offline --all-targets` → clean.

- [ ] **Step 7: Commit.**
```bash
git add src/session.rs src/cli.rs src/main.rs tests/session_integration.rs
git commit -m "feat: anka session quick actions (new/kill/promote/switch/last/rename)"
```

---

### Task 2: Switcher items + fuzzy matcher (pure)

**Files:**
- Create: `src/switcher/mod.rs` (only `mod state; pub use ...` for now), `src/switcher/state.rs`
- Modify: `src/main.rs` (`mod switcher;`)

**Interfaces:**
- Produces: `enum Item { Live(String), Snapshot(String), Window { session: String, index: u32, name: String }, Zoxide(std::path::PathBuf) }`; `fn item_label(&Item) -> String`; `fn item_key(&Item) -> String` (the fuzzy-match target); `fn build_session_items(live: &[String], snapshot: &[String]) -> Vec<Item>`; `fn fuzzy_score(query: &str, hay: &str) -> Option<i32>` (None = no match, higher = better).

- [ ] **Step 1: Write failing unit tests** in `src/switcher/state.rs` (`#[cfg(test)]`):

```rust
#[test]
fn build_merges_and_dedupes() {
    let items = build_session_items(&["KENP".into(), "Tor".into()], &["KENP".into(), "old".into()]);
    let labels: Vec<String> = items.iter().map(item_label).collect();
    // live first (with (live)), then snapshot-only (with (snapshot)); KENP not duplicated
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
```

- [ ] **Step 2: Run, verify fail.** `cargo test --offline switcher::state` → FAIL.

- [ ] **Step 3: Implement** the model + matcher in `src/switcher/state.rs`:

```rust
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

pub fn item_key(i: &Item) -> String {
    match i {
        Item::Live(n) | Item::Snapshot(n) => n.clone(),
        Item::Window { session, name, .. } => format!("{session} {name}"),
        Item::Zoxide(p) => p.display().to_string(),
    }
}

pub fn build_session_items(live: &[String], snapshot: &[String]) -> Vec<Item> {
    let mut items: Vec<Item> = live.iter().cloned().map(Item::Live).collect();
    for s in snapshot {
        if !live.iter().any(|l| l == s) {
            items.push(Item::Snapshot(s.clone()));
        }
    }
    items
}

/// Case-insensitive subsequence match. Returns None if `query` is not a
/// subsequence of `hay`; otherwise a score where contiguous and earlier matches
/// score higher (prefix matches rank best).
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
```
Add to `src/switcher/mod.rs`:
```rust
mod state;
pub use state::*;
```
and `mod switcher;` to `src/main.rs`.

- [ ] **Step 4: Run, verify pass.** `cargo test --offline switcher::state` → PASS. `cargo clippy` clean.

- [ ] **Step 5: Commit.**
```bash
git add src/switcher/ src/main.rs
git commit -m "feat: switcher item model + fuzzy matcher (pure)"
```

---

### Task 3: Switcher state machine (`apply`)

**Files:**
- Modify: `src/switcher/state.rs`
- Test: same file `#[cfg(test)]`

**Interfaces:**
- Produces:
```rust
pub enum Mode { Sessions, Windows, Zoxide }
pub enum Key { Up, Down, Enter, Tab, Backspace, Char(char), New, Rename, Delete, Cancel }
pub enum Stay { Kill(Item), Rename { item: Item, to: String } }      // keep TUI open, reload
pub enum Exit { Activate(Item), NewSession(String), Cancel }         // close popup
pub enum Step { Redraw, PreviewChanged, Stay(Stay), Exit(Exit) }
pub struct State { /* see impl */ }
impl State {
    pub fn new(items: Vec<Item>, mode: Mode) -> Self;
    pub fn visible(&self) -> Vec<&Item>;     // filtered items
    pub fn selected(&self) -> Option<&Item>;
    pub fn apply(&mut self, key: Key) -> Step;
    pub fn prompt(&self) -> Option<(&'static str, &str)>; // (label, buffer) when prompting
}
```
- Consumes: `Item`, `fuzzy_score`, `item_key` from Task 2.

- [ ] **Step 1: Write failing tests** in `state.rs`:

```rust
#[cfg(test)]
mod apply_tests {
    use super::*;
    fn st() -> State {
        State::new(vec![Item::Live("KENP".into()), Item::Live("dev".into()), Item::Snapshot("old".into())], Mode::Sessions)
    }
    #[test]
    fn typing_filters_and_resets_cursor() {
        let mut s = st();
        assert!(matches!(s.apply(Key::Char('d')), Step::Redraw));
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
        assert!(matches!(s.mode(), Mode::Windows));
    }
    #[test]
    fn new_prompt_then_enter_creates_session() {
        let mut s = st();
        s.apply(Key::Char('w')); // query "w" prefilled into prompt
        s.apply(Key::New);
        assert!(s.prompt().is_some());
        s.apply(Key::Char('x'));
        match s.apply(Key::Enter) {
            Step::Exit(Exit::NewSession(name)) => assert_eq!(name, "wx"),
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
}
```
Also add a `pub fn mode(&self) -> &Mode` accessor used by tests.

- [ ] **Step 2: Run, verify fail.**

- [ ] **Step 3: Implement** `State` + `apply` in `state.rs`. Key rules:
  - `query` edits (`Char`, `Backspace`) re-run the filter (`refilter`), clamp cursor, return `Redraw`. While `prompt.is_some()`, `Char`/`Backspace` edit the prompt buffer instead and return `Redraw`.
  - `Up`/`Down` move cursor within `visible()`, return `PreviewChanged`.
  - `Tab` cycles `Sessions→Windows→Zoxide→Sessions` (the loop reloads items for the new mode; `apply` just sets mode + returns `Redraw`). Items for the new mode are injected by the I/O layer via `set_items`.
  - `Enter`: if prompting → finish (`Exit::NewSession(buf)` for New, `Stay::Rename` for Rename); else `Exit::Activate(selected.clone())` (or `Exit::Cancel` if list empty).
  - `New`: enter prompt mode (label "New session:", buffer = current `query`).
  - `Rename`: enter prompt (label "Rename to:", buffer = selected session name); only valid on Live/Snapshot.
  - `Delete`: on Live → `Stay::Kill(item)`; otherwise `Redraw` (no-op for now; snapshot disk-removal is out of scope).
  - `Cancel`: if prompting → leave prompt, `Redraw`; else `Exit::Cancel`.

```rust
pub enum Mode { Sessions, Windows, Zoxide }
pub enum Key { Up, Down, Enter, Tab, Backspace, Char(char), New, Rename, Delete, Cancel }
#[derive(Debug)] pub enum Stay { Kill(Item), Rename { item: Item, to: String } }
#[derive(Debug)] pub enum Exit { Activate(Item), NewSession(String), Cancel }
#[derive(Debug)] pub enum Step { Redraw, PreviewChanged, Stay(Stay), Exit(Exit) }

enum Prompt { New(String), Rename(String) }

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
        let mut s = State { mode, items, filtered: vec![], query: String::new(), cursor: 0, prompt: None };
        s.refilter();
        s
    }
    pub fn mode(&self) -> &Mode { &self.mode }
    pub fn set_items(&mut self, items: Vec<Item>) { self.items = items; self.cursor = 0; self.refilter(); }
    pub fn visible(&self) -> Vec<&Item> { self.filtered.iter().map(|&i| &self.items[i]).collect() }
    pub fn selected(&self) -> Option<&Item> { self.filtered.get(self.cursor).map(|&i| &self.items[i]) }
    pub fn prompt(&self) -> Option<(&'static str, &str)> {
        match &self.prompt {
            Some(Prompt::New(b)) => Some(("New session:", b)),
            Some(Prompt::Rename(b)) => Some(("Rename to:", b)),
            None => None,
        }
    }
    fn refilter(&mut self) {
        let mut scored: Vec<(usize, i32)> = self.items.iter().enumerate()
            .filter_map(|(i, it)| fuzzy_score(&self.query, &item_key(it)).map(|sc| (i, sc)))
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        if self.cursor >= self.filtered.len() { self.cursor = self.filtered.len().saturating_sub(1); }
    }
    pub fn apply(&mut self, key: Key) -> Step {
        if self.prompt.is_some() {
            return self.apply_prompt(key);
        }
        match key {
            Key::Char(c) => { self.query.push(c); self.refilter(); Step::Redraw }
            Key::Backspace => { self.query.pop(); self.refilter(); Step::Redraw }
            Key::Up => { self.cursor = self.cursor.saturating_sub(1); Step::PreviewChanged }
            Key::Down => { if self.cursor + 1 < self.filtered.len() { self.cursor += 1; } Step::PreviewChanged }
            Key::Tab => { self.mode = match self.mode { Mode::Sessions => Mode::Windows, Mode::Windows => Mode::Zoxide, Mode::Zoxide => Mode::Sessions }; Step::Redraw }
            Key::Enter => match self.selected().cloned() {
                Some(it) => Step::Exit(Exit::Activate(it)),
                None if !self.query.is_empty() => Step::Exit(Exit::NewSession(self.query.clone())),
                None => Step::Exit(Exit::Cancel),
            },
            Key::New => { self.prompt = Some(Prompt::New(self.query.clone())); Step::Redraw }
            Key::Rename => match self.selected() {
                Some(Item::Live(n)) | Some(Item::Snapshot(n)) => { self.prompt = Some(Prompt::Rename(n.clone())); Step::Redraw }
                _ => Step::Redraw,
            },
            Key::Delete => match self.selected().cloned() {
                Some(it @ Item::Live(_)) => Step::Stay(Stay::Kill(it)),
                _ => Step::Redraw,
            },
            Key::Cancel => Step::Exit(Exit::Cancel),
        }
    }
    fn apply_prompt(&mut self, key: Key) -> Step {
        let buf = match self.prompt.as_mut().unwrap() { Prompt::New(b) | Prompt::Rename(b) => b };
        match key {
            Key::Char(c) => { buf.push(c); Step::Redraw }
            Key::Backspace => { buf.pop(); Step::Redraw }
            Key::Cancel => { self.prompt = None; Step::Redraw }
            Key::Enter => {
                let p = self.prompt.take().unwrap();
                match p {
                    Prompt::New(name) => Step::Exit(Exit::NewSession(name)),
                    Prompt::Rename(to) => match self.selected().cloned() {
                        Some(item) => Step::Stay(Stay::Rename { item, to }),
                        None => Step::Redraw,
                    },
                }
            }
            _ => Step::Redraw,
        }
    }
}
```

- [ ] **Step 4: Run, verify pass.** `cargo test --offline switcher` → PASS. clippy clean.

- [ ] **Step 5: Commit.**
```bash
git add src/switcher/state.rs
git commit -m "feat: switcher state machine (apply: filter/nav/mode/prompt/effects)"
```

---

### Task 4: Terminal layer — key parser + raw mode (`term.rs`)

**Files:**
- Create: `src/switcher/term.rs`
- Modify: `src/switcher/mod.rs` (`mod term;`)
- Test: `term.rs` `#[cfg(test)]` (parser only)

**Interfaces:**
- Produces: `fn parse_key(buf: &[u8]) -> Option<(Key, usize)>` (None = need more bytes; usize = bytes consumed); `struct RawMode` (RAII: `enter() -> Result<RawMode>`, restores on `drop` and via explicit `restore()`); `fn term_size() -> (u16, u16)` (cols, rows; sensible default 80×24); ANSI helpers `fn clear() -> &str`, `fn move_to(row, col) -> String`, `fn hide_cursor()/show_cursor()`.
- Consumes: `Key` from Task 3.

- [ ] **Step 1: Write failing parser tests** in `term.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::switcher::Key;
    #[test]
    fn parses_printable_and_controls() {
        assert!(matches!(parse_key(b"a"), Some((Key::Char('a'), 1))));
        assert!(matches!(parse_key(b"\r"), Some((Key::Enter, 1))));
        assert!(matches!(parse_key(b"\n"), Some((Key::Enter, 1))));
        assert!(matches!(parse_key(&[0x7f]), Some((Key::Backspace, 1))));
        assert!(matches!(parse_key(b"\t"), Some((Key::Tab, 1))));
        assert!(matches!(parse_key(&[0x0e]), Some((Key::New, 1)))); // ^N
        assert!(matches!(parse_key(&[0x12]), Some((Key::Rename, 1)))); // ^R
        assert!(matches!(parse_key(&[0x18]), Some((Key::Delete, 1)))); // ^X
        assert!(matches!(parse_key(&[0x03]), Some((Key::Cancel, 1)))); // ^C
        assert!(matches!(parse_key(&[0x1b]), Some((Key::Cancel, 1)))); // lone ESC
    }
    #[test]
    fn parses_arrow_escape_sequences() {
        assert!(matches!(parse_key(b"\x1b[A"), Some((Key::Up, 3))));
        assert!(matches!(parse_key(b"\x1b[B"), Some((Key::Down, 3))));
    }
    #[test]
    fn incomplete_escape_needs_more() {
        assert_eq!(parse_key(b"\x1b["), None); // wait for the final byte
    }
}
```

- [ ] **Step 2: Run, verify fail.**

- [ ] **Step 3: Implement** `term.rs`:

```rust
//! Raw-mode terminal I/O for the switcher (dependency-free, via `stty`).

use std::io::Write;
use std::process::Command;
use crate::switcher::Key;

pub fn parse_key(buf: &[u8]) -> Option<(Key, usize)> {
    match buf.first()? {
        b'\r' | b'\n' => Some((Key::Enter, 1)),
        b'\t' => Some((Key::Tab, 1)),
        0x7f | 0x08 => Some((Key::Backspace, 1)),
        0x0e => Some((Key::New, 1)),
        0x12 => Some((Key::Rename, 1)),
        0x18 => Some((Key::Delete, 1)),
        0x03 => Some((Key::Cancel, 1)),
        0x1b => {
            if buf.len() == 1 { return Some((Key::Cancel, 1)); }   // lone ESC
            if buf[1] == b'[' {
                if buf.len() < 3 { return None; }                  // need final byte
                return match buf[2] {
                    b'A' => Some((Key::Up, 3)),
                    b'B' => Some((Key::Down, 3)),
                    _ => Some((Key::Cancel, 3)),                    // ignore other CSI
                };
            }
            Some((Key::Cancel, 1))
        }
        &b if b >= 0x20 => {
            // decode one UTF-8 char
            let s = std::str::from_utf8(buf).ok()?;
            let c = s.chars().next()?;
            Some((Key::Char(c), c.len_utf8()))
        }
        _ => Some((Key::Cancel, 1)),
    }
}

pub struct RawMode { saved: String }
impl RawMode {
    pub fn enter() -> anyhow::Result<RawMode> {
        let saved = stty(&["-g"])?;            // dump current settings
        let _ = stty(&["raw", "-echo"]);
        print!("\x1b[?25l");                    // hide cursor
        std::io::stdout().flush().ok();
        Ok(RawMode { saved })
    }
    pub fn restore(&self) {
        let _ = stty(&[&self.saved]);
        print!("\x1b[?25h\x1b[2J\x1b[H");        // show cursor, clear
        std::io::stdout().flush().ok();
    }
}
impl Drop for RawMode { fn drop(&mut self) { self.restore(); } }

fn stty(args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("stty").args(args).output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn term_size() -> (u16, u16) {
    if let Ok(s) = stty(&["size"]) {
        let mut it = s.split_whitespace();
        if let (Some(r), Some(c)) = (it.next(), it.next()) {
            if let (Ok(r), Ok(c)) = (r.parse(), c.parse()) { return (c, r); }
        }
    }
    (80, 24)
}

pub fn clear() -> &'static str { "\x1b[2J\x1b[H" }
pub fn move_to(row: u16, col: u16) -> String { format!("\x1b[{row};{col}H") }
```
Add `mod term;` to `src/switcher/mod.rs`.

- [ ] **Step 4: Run, verify pass.** `cargo test --offline switcher::term` → PASS. clippy clean.

- [ ] **Step 5: Commit.**
```bash
git add src/switcher/term.rs src/switcher/mod.rs
git commit -m "feat: switcher terminal layer (key parser + raw mode via stty)"
```

---

### Task 5: Switcher loop + fallback + CLI wiring

**Files:**
- Modify: `src/switcher/mod.rs` (the `pick`/`run` entry, loop, fallback, effect dispatch)
- Modify: `src/cli.rs` (add `Switch`), `src/main.rs` (dispatch `Switch`; route `Pick` → switcher)
- Modify: `src/tui.rs` → fold its numbered-menu into the switcher fallback (or call from mod.rs)
- Test: `tests/switch_integration.rs`

**Interfaces:**
- Produces: `switcher::run() -> Result<()>` (the `anka switch` entry); reused by `anka pick`.
- Consumes: Task 2/3/4 items + state + term; `restore::load_snapshot`, `restore::restore_one`, `store::last_name`, `session::*` helpers (make `current_session`/`sessions` reusable or duplicate minimal), `tmux::*`.

- [ ] **Step 1: CLI + dispatch.** `src/cli.rs`: add `/// Interactive session switcher\n    Switch,`. `src/main.rs`: `Cmd::Switch => switcher::run(),` and change `Cmd::Pick => switcher::run(),` (pick now opens the switcher).

- [ ] **Step 2: Write failing integration tests** (`tests/switch_integration.rs`) driving the **non-tty fallback** (piped stdin → numbered menu):

```rust
mod common;
use common::*;

#[test]
fn switch_fallback_switches_to_chosen_live_session() {
    if !has_tmux() { eprintln!("skip"); return; }
    let s = Server::start("sw-live");
    s.tmux(&["new-session", "-d", "-s", "alpha", "-x", "200", "-y", "50"]);
    s.tmux(&["new-session", "-d", "-s", "beta", "-x", "200", "-y", "50"]);
    // items: alpha, beta, scratch (sorted by tmux order) — pick "beta"
    let out = s.anka_stdin(&["switch"], "beta\n"); // fallback accepts a name or index
    assert!(out.status.success(), "{}", err(&out));
}

#[test]
fn switch_fallback_restores_snapshot_session() {
    if !has_tmux() { eprintln!("skip"); return; }
    let s = Server::start("sw-snap");
    s.tmux(&["new-session", "-d", "-s", "gone", "-x", "200", "-y", "50"]);
    assert!(s.anka(&["save"]).status.success());
    s.tmux(&["kill-session", "-t", "gone"]);
    let out = s.anka_stdin(&["switch"], "gone\n");
    assert!(out.status.success(), "{}", err(&out));
    assert!(sessions(&s.socket).contains(&"gone".to_string()));
}
```
(The fallback parses the line as an index *or* a session-name substring; document both. Reuse `anka_stdin` from `tests/common/mod.rs`.)

- [ ] **Step 3: Run, verify fail.**

- [ ] **Step 4: Implement `src/switcher/mod.rs`:**
  - `run()`:
    1. `tmux::server_running()` else bail.
    2. Build sessions-mode items: `live = list-sessions`, `snapshot = last snapshot session names not live`.
    3. If `std::io::IsTerminal` on stdin **and** stdout → `interactive(items)`, else `fallback(items)`.
  - `fallback(items)`: print the numbered list (the v0.7.0 `tui.rs` format) over Sessions items; read one line; resolve to an item by **index** or **name substring/fuzzy** (`fuzzy_score`), then `activate(item)`.
  - `interactive(items)`: `RawMode::enter()`; loop: render(state, preview), read bytes → `parse_key` → `state.apply` → on `Redraw` re-render, on `PreviewChanged` recompute preview + render, on `Stay` perform op (kill/rename) then reload items + render, on `Exit` restore terminal and perform the exit effect (or cancel). On `Tab`, reload items for the new mode via `mode_items(mode)`.
  - `mode_items(mode)`: Sessions → as above; Windows → `list-windows -a -F '#{session_name}\t#{window_index}\t#{window_name}'` → `Item::Window`; Zoxide → `zoxide query -l` lines → `Item::Zoxide` (empty if zoxide missing or `@anka-zoxide off`).
  - `activate(item)`:
    - `Live(n)` → `switch-client -t n`.
    - `Snapshot(n)` → `restore::load_snapshot(last)?; restore::restore_one(&snap, n)?`.
    - `Window{session,index,..}` → `switch-client -t session ; select-window -t session:index`.
    - `Zoxide(path)` → `session::new_in_dir(basename, path)` (new-session -c path, switch).
  - `Exit::NewSession(name)` → `session`-style new; `Stay::Kill(Live(n))` → `kill-session -t n`; `Stay::Rename{item,to}` → `rename-session -t <name> to`.
  - `preview(item, width, height)`:
    - Live/Window → `tmux capture-pane -ep -t <target> -S -<n>` (last height-ish lines).
    - Snapshot → structural summary (windows/panes + first cwd) from the loaded snapshot.
    - Zoxide → path + first few `read_dir` entries.
  - Respect `@anka-switch-preview` (skip preview column when off or terminal too narrow, e.g. `cols < 80`).

  Render: left column = filtered labels (highlight `cursor`, dim `(live)/(snapshot)` tags), bottom = `query` or active prompt + a footer `↑↓ select · ⏎ go · ^n new · ^r rename · ^x kill · Tab mode · esc cancel`; right column = preview (when enabled).

  Implementation note: reuse `session.rs` by exposing `pub(crate) fn new_named(name)`, `pub(crate) fn new_in_dir(name, dir)`, so the switcher and `anka session` share one code path. Add these to `session.rs`.

- [ ] **Step 5: Run, verify pass.** `cargo test --offline` (whole suite) → PASS. `cargo clippy --offline --all-targets` → clean.

- [ ] **Step 6: Manual smoke test** in a real tmux:
```bash
cargo build --release && cp target/release/anka bin/anka.new && mv -f bin/anka.new bin/anka
tmux display-popup -E "$PWD/bin/anka switch"
```
Verify: arrows move, typing filters, Enter switches, Tab cycles modes, ^n/^r/^x work, preview shows, Esc cancels, terminal restored after.

- [ ] **Step 7: Commit.**
```bash
git add src/switcher/mod.rs src/cli.rs src/main.rs src/session.rs src/tui.rs tests/switch_integration.rs
git commit -m "feat: interactive session switcher with non-tty fallback (anka switch)"
```

---

### Task 6: Config options + anka.tmux keybindings

**Files:**
- Modify: `src/config.rs` (add `switch_preview: bool`, `zoxide: bool`)
- Modify: `anka.tmux`

**Interfaces:**
- Consumes: `tmux show-options -gqv` (existing `opt_bool`).

- [ ] **Step 1:** In `src/config.rs`, add fields + loads:
```rust
    pub switch_preview: bool,
    pub zoxide: bool,
```
```rust
            switch_preview: opt_bool("@anka-switch-preview", true),
            zoxide: opt_bool("@anka-zoxide", true),
```
Use them in `switcher::mod` (`Config::load()`), replacing any direct option reads.

- [ ] **Step 2:** In `anka.tmux`, after the existing key reads, add:
```bash
SWITCH_KEY="$(opt @anka-switch-key)";        SWITCH_KEY="${SWITCH_KEY:-s}"
NEW_KEY="$(opt @anka-new-key)";              NEW_KEY="${NEW_KEY:-C}"
KILL_KEY="$(opt @anka-kill-key)";            KILL_KEY="${KILL_KEY:-X}"
PROMOTE_KEY="$(opt @anka-promote-key)";      PROMOTE_KEY="${PROMOTE_KEY:-@}"
SWITCH_NAME_KEY="$(opt @anka-switch-name-key)"; SWITCH_NAME_KEY="${SWITCH_NAME_KEY:-g}"
LAST_KEY="$(opt @anka-last-key)";            LAST_KEY="${LAST_KEY:-S}"
```
and the binds:
```bash
tmux bind-key "$SWITCH_KEY"      display-popup -E "$BINARY switch"
tmux bind-key "$NEW_KEY"         command-prompt -p "New session:" "run-shell \"$BINARY session new '%%'\""
tmux bind-key "$KILL_KEY"        run-shell "$BINARY session kill"
tmux bind-key "$PROMOTE_KEY"     command-prompt -p "Promote pane to session:" "run-shell \"$BINARY session promote '%%'\""
tmux bind-key "$SWITCH_NAME_KEY" command-prompt -p "Switch to session:" "run-shell \"$BINARY session switch '%%'\""
tmux bind-key "$LAST_KEY"        run-shell "$BINARY session last"
```
Keep the existing `PICK_KEY` bind (now opens the switcher via `$BINARY pick`).

- [ ] **Step 3:** `cargo build --offline` → OK; `cargo clippy` clean.

- [ ] **Step 4: Commit.**
```bash
git add src/config.rs anka.tmux
git commit -m "feat: switcher/sessionist keybindings + @anka-switch-preview/@anka-zoxide"
```

---

### Task 7: Docs + version bump + binary

**Files:**
- Modify: `README.md`, `docs/DESIGN.md`, `Cargo.toml`, `Cargo.lock`
- Build: `bin/anka`

- [ ] **Step 1:** `Cargo.toml` version `0.7.0` → `0.8.0`. Run `cargo build --release --offline` (updates `Cargo.lock`, builds binary).

- [ ] **Step 2:** README — add a **Session management** section (switcher `prefix+s`, the mode/keys, sessionist binds), add the new config rows and CLI lines, and note `prefix+P`/`pick` now open the switcher.

- [ ] **Step 3:** DESIGN — add a "Session management" section summarising the state-machine/IO split + fallback; add a v0.8.0 roadmap line.

- [ ] **Step 4:** Install the binary (avoid ETXTBSY):
```bash
cp target/release/anka bin/anka.new && mv -f bin/anka.new bin/anka && ./bin/anka --version
```

- [ ] **Step 5: Commit + tag.**
```bash
git add Cargo.toml Cargo.lock README.md docs/DESIGN.md
git commit -m "chore(release): v0.8.0 — native session management"
git tag -a v0.8.0 -m "anka v0.8.0 — session switcher + sessionist actions"
```

- [ ] **Step 6: Push** (canonical repo, SSH):
```bash
git push origin main && git push origin v0.8.0
```
Verify CI: `gh run list -R kenanpelit/tmux-anka -L 2` and assets: `gh release view v0.8.0 -R kenanpelit/tmux-anka --json assets --jq '.assets[].name'`.

---

### Task 8: Migration — remove sessionist + sessionx (in `~/.cachy`)

**Files:**
- Modify: `~/.cachy/modules/tmux/dotfiles/tmux/tmux.conf`
- Remove: gitlinks + dirs `tmux-sessionist`, `tmux-sessionx`
- Modify: anka gitlink → v0.8.0

- [ ] **Step 1:** Grep for custom settings to migrate/remove:
```bash
grep -nE 'sessionx|sessionist' ~/.cachy/modules/tmux/dotfiles/tmux/tmux.conf
```
Remove the two `@plugin` lines and any `@sessionx-*`/`@sessionist-*` settings (migrate a custom key to the matching `@anka-*-key` if found).

- [ ] **Step 2:** Sync the gitlink to v0.8.0 and remove the two plugins:
```bash
cd ~/.cachy/modules/tmux/dotfiles/tmux/plugins/tmux-anka && git fetch origin --tags && git pull --ff-only origin main
cd /home/kenan/.cachy
git rm --cached -q modules/tmux/dotfiles/tmux/plugins/tmux-sessionist modules/tmux/dotfiles/tmux/plugins/tmux-sessionx
rm -rf modules/tmux/dotfiles/tmux/plugins/tmux-sessionist modules/tmux/dotfiles/tmux/plugins/tmux-sessionx
git add modules/tmux/dotfiles/tmux/plugins/tmux-anka modules/tmux/dotfiles/tmux/tmux.conf
```

- [ ] **Step 3: Commit + push** the dotfiles repo:
```bash
git commit -m "tmux: replace sessionist+sessionx with native anka session management (v0.8.0)"
git push origin main
```

- [ ] **Step 4: Verify** the plugin list now matches the dir (no orphans), and that reloading tmux (`tmux source ~/.config/tmux/tmux.conf` or restart) binds `prefix+s` to the switcher with no errors.

---

## Self-Review

- **Spec coverage:** switcher TUI (T2–T5), state-machine/IO split (T2–T5), fallback (T5), live+snapshot+windows+zoxide modes (T5), inline new/rename/delete (T3+T5), preview (T5), sessionist actions (T1), keybindings+config (T6), `pick`→switch redirect (T5), migration/remove plugins (T8), docs+version+release (T7). All covered.
- **Placeholders:** none — code given for all logic; I/O glue (T5 render/loop) is specified with exact tmux commands + verified by integration tests (fallback) and a manual smoke test (interactive).
- **Type consistency:** `Item`, `Key`, `Step`/`Stay`/`Exit`, `State::{new,apply,visible,selected,mode,set_items,prompt}`, `parse_key`, `RawMode`, `fuzzy_score`, `item_label`, `item_key`, `build_session_items` used consistently across tasks.

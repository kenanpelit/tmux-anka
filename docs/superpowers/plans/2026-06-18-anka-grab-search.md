# anka grab + anka search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `anka grab` (extract tokens → copy/paste/open) and `anka search` (scrollback fuzzy-search → jump), replacing extrakto + tmux-fuzzback.

**Architecture:** Two subcommands reusing anka's picker, pane-capture, `url` (extract/open) and `clip` (copy). Two-step launch like `anka url` (run-shell captures `#{pane_id}` → popup picker; actions target the original pane). The picker gains `pick_ex`/`Hit` + `Key::Ctrl(char)` for grab's Tab-cycle + action keys.

**Tech Stack:** Rust (clap, anyhow), tmux CLI, reuse of `src/switcher`, `src/url.rs`, `src/clip.rs`.

## Global Constraints

- Edit canonical `~/.kod/tmux/tmux-anka`; `.cachy` embeds it as a gitlink (no `.gitmodules` for anka); push over SSH.
- Deps stay clap + anyhow (+ existing serde/chrono); no `fzf` runtime dep.
- Linux-only; tmux 3.x.
- `cargo test` + `cargo clippy --all-targets -- -D warnings` clean before every commit.
- Stop the live daemon (`pkill -x anka`) before replacing `bin/anka`.

---

### Task 1: `Key::Ctrl(char)` + parse_key arm

**Files:**
- Modify: `src/switcher/state.rs` (Key enum)
- Modify: `src/switcher/term.rs` (parse_key + test)

**Interfaces:**
- Produces: `Key::Ctrl(char)` variant; `parse_key` maps unhandled control bytes `0x01..=0x1a` to it.

- [ ] **Step 1: Add the variant.** In `src/switcher/state.rs`, in `pub enum Key` (already derives `Clone, Copy, Debug, PartialEq`), add after `Delete,`:

```rust
    Ctrl(char),
```

- [ ] **Step 2: Write the failing test.** In `src/switcher/term.rs`, append:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_letters_map_to_ctrl_variant() {
        assert_eq!(parse_key(&[0x16]), Some((Some(Key::Ctrl('v')), 1))); // ^V
        assert_eq!(parse_key(&[0x0f]), Some((Some(Key::Ctrl('o')), 1))); // ^O
        // existing specific bindings are unchanged:
        assert_eq!(parse_key(&[0x12]), Some((Some(Key::Rename), 1))); // ^R
        assert_eq!(parse_key(&[0x10]), Some((Some(Key::Up), 1))); // ^P
    }
}
```

- [ ] **Step 3: Run test to verify it fails.**

Run: `cargo test -p anka term::tests` (or `cargo test ctrl_letters`)
Expected: FAIL — `Key::Ctrl` not found / arm missing.

- [ ] **Step 4: Add the parse arm.** In `src/switcher/term.rs` `parse_key`, replace the final fallthrough `_ => Some((None, 1))` with:

```rust
        0x01..=0x1a => Some((Some(Key::Ctrl((b0 + 0x60) as char)), 1)),
        _ => Some((None, 1)), // other control bytes: consume + ignore
```

- [ ] **Step 5: Run tests.**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: PASS; no warnings (the new `Ctrl` is consumed in Task 2).

- [ ] **Step 6: Commit.**

```bash
git add src/switcher/state.rs src/switcher/term.rs
git commit -m "feat(term): Key::Ctrl(char) for unmapped control keys"
```

---

### Task 2: picker `pick_ex` + `Hit` (refactor `pick` onto a shared loop)

**Files:**
- Modify: `src/picker.rs`

**Interfaces:**
- Consumes: `Key::Ctrl` (Task 1).
- Produces:
  - `pub enum Hit { Enter(usize), Ctrl(char, usize), Tab, Cancel }`
  - `pub fn pick(items: &[String], title: &str) -> anyhow::Result<Option<usize>>` (unchanged signature)
  - `pub fn pick_ex(items: &[String], title: &str) -> anyhow::Result<Hit>`
  - `pub fn pick_str(items: &[String], title: &str) -> anyhow::Result<Option<String>>` (unchanged)

- [ ] **Step 1: Replace the `pick`/`pick_str` block** in `src/picker.rs` (the `pub fn pick`, `pub fn pick_str`) with the shared loop. Keep `refilter`, `render`, and the consts as-is:

```rust
#[derive(Debug, PartialEq)]
pub enum Hit {
    Enter(usize),
    Ctrl(char, usize),
    Tab,
    Cancel,
}

fn run_picker(items: &[String], title: &str, full: bool) -> Result<Hit> {
    if items.is_empty() {
        return Ok(Hit::Cancel);
    }
    let raw = RawMode::enter()?;
    let mut query = String::new();
    let mut cursor = 0usize;
    let mut filtered: Vec<usize> = (0..items.len()).collect();

    render(items, &filtered, cursor, &query, title);
    let mut stdin = io::stdin();
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 32];
    loop {
        let n = stdin.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        while let Some((maybe, used)) = term::parse_key(&buf) {
            buf.drain(..used);
            let Some(key) = maybe else { continue };
            match key {
                Key::Cancel => {
                    raw.restore();
                    return Ok(Hit::Cancel);
                }
                Key::Enter => {
                    if let Some(&i) = filtered.get(cursor) {
                        raw.restore();
                        return Ok(Hit::Enter(i));
                    }
                }
                Key::Digit(d) if d >= 1 && d - 1 < filtered.len() => {
                    raw.restore();
                    return Ok(Hit::Enter(filtered[d - 1]));
                }
                Key::Tab if full => {
                    raw.restore();
                    return Ok(Hit::Tab);
                }
                Key::Ctrl(c) if full => {
                    if let Some(&i) = filtered.get(cursor) {
                        raw.restore();
                        return Ok(Hit::Ctrl(c, i));
                    }
                }
                Key::Up => cursor = cursor.saturating_sub(1),
                Key::Down if cursor + 1 < filtered.len() => cursor += 1,
                Key::Char(c) => {
                    query.push(c);
                    filtered = refilter(items, &query);
                    cursor = 0;
                }
                Key::Backspace => {
                    query.pop();
                    filtered = refilter(items, &query);
                    cursor = 0;
                }
                _ => {}
            }
            render(items, &filtered, cursor, &query, title);
        }
    }
    raw.restore();
    Ok(Hit::Cancel)
}

/// Interactive pick; returns the chosen index, or `None` on cancel.
pub fn pick(items: &[String], title: &str) -> Result<Option<usize>> {
    match run_picker(items, title, false)? {
        Hit::Enter(i) => Ok(Some(i)),
        _ => Ok(None),
    }
}

/// Like `pick`, but also returns Tab (cycle) and Ctrl-key accepts.
pub fn pick_ex(items: &[String], title: &str) -> Result<Hit> {
    run_picker(items, title, true)
}

/// Convenience: return the chosen item itself.
pub fn pick_str(items: &[String], title: &str) -> Result<Option<String>> {
    Ok(pick(items, title)?.map(|i| items[i].clone()))
}
```

- [ ] **Step 2: Build + test (refactor safety net: url/menu still work).**

Run: `cargo build && cargo test && cargo clippy --all-targets -- -D warnings`
Expected: builds; all existing tests pass; no warnings.

- [ ] **Step 3: Commit.**

```bash
git add src/picker.rs
git commit -m "feat(picker): pick_ex/Hit (Tab cycle + Ctrl accepts); pick shares the loop"
```

---

### Task 3: grab extractors (pure)

**Files:**
- Create: `src/grab.rs`
- Modify: `src/main.rs` (add `mod grab;`)

**Interfaces:**
- Consumes: `crate::url::extract_urls` (already `pub`).
- Produces:
  - `pub const FILTERS: &[&str]` = `["all", "url", "path", "word", "line"]`
  - `pub fn parse_paths(text: &str) -> Vec<String>`
  - `pub fn parse_words(text: &str) -> Vec<String>`
  - `pub fn parse_lines(text: &str) -> Vec<String>`
  - `pub fn tokens_for(filter: &str, text: &str) -> Vec<String>`

- [ ] **Step 1: Write the extractors + failing tests.** Create `src/grab.rs`:

```rust
//! `anka grab` — extract tokens from the pane and copy/paste/open (replaces
//! extrakto). Filters are cycled with Tab; actions: Enter=copy, ^v=paste, ^o=open.

use std::collections::HashSet;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_trims_and_dedupes() {
        assert_eq!(parse_lines("  a \n\n a \n b "), vec!["a", "b"]);
    }

    #[test]
    fn words_min_len_and_trim() {
        let w = parse_words("(foo) a bar, foo");
        assert_eq!(w, vec!["foo", "bar"]); // "a" too short; "foo" deduped
    }

    #[test]
    fn paths_detected_not_plain_words() {
        let p = parse_paths("see /etc/hosts and ./src/main.rs not justaword or github.com/x");
        assert_eq!(p, vec!["/etc/hosts", "./src/main.rs"]);
    }

    #[test]
    fn all_unions_url_path_word() {
        let t = tokens_for("all", "go https://x.com/a /tmp/f.txt hello");
        assert_eq!(t[0], "https://x.com/a");
        assert!(t.contains(&"/tmp/f.txt".to_string()));
        assert!(t.contains(&"hello".to_string()));
    }
}
```

- [ ] **Step 2: Register module + run tests to verify they pass.** Add `mod grab;` to `src/main.rs`.

Run: `cargo test grab:: && cargo clippy --all-targets -- -D warnings`
Expected: PASS; no warnings.

- [ ] **Step 3: Commit.**

```bash
git add src/grab.rs src/main.rs
git commit -m "feat(grab): token extractors (url/path/word/line/all)"
```

---

### Task 4: grab flow + `Grab` subcommand (+ expose `clip::copy`, `url::open`)

**Files:**
- Modify: `src/clip.rs` (extract `pub(crate) fn copy`)
- Modify: `src/url.rs` (`open` → `pub(crate)`)
- Modify: `src/grab.rs` (the popup flow)
- Modify: `src/cli.rs`, `src/main.rs`

**Interfaces:**
- Consumes: `picker::pick_ex`, `Hit`, `tokens_for`, `FILTERS`, `crate::tmux`.
- Produces: `pub(crate) fn clip::copy(data: &[u8], primary: bool)`; `pub(crate) fn url::open(url: &str)`; `pub fn grab::run(pane: Option<&str>, source: Option<&str>) -> Result<()>`.

- [ ] **Step 1: Expose `clip::copy`.** In `src/clip.rs`, refactor `run` to delegate. Replace the body of `pub fn run(primary: bool)` so the env-dispatch lives in a reusable `copy`:

```rust
pub(crate) fn copy(data: &[u8], primary: bool) {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let x11 = std::env::var_os("DISPLAY").is_some();
    match backend(wayland, x11) {
        Backend::Wayland => copy_via("wl-copy", &wl_args(primary), data),
        Backend::X11 => copy_via("xclip", &xclip_args(primary), data),
        Backend::Osc52 => {
            if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
                let _ = tty.write_all(osc52(data).as_bytes());
            }
        }
    }
}

pub fn run(primary: bool) -> Result<()> {
    let mut data = Vec::new();
    io::stdin().read_to_end(&mut data)?;
    copy(&data, primary);
    Ok(())
}
```

- [ ] **Step 2: Expose `url::open`.** In `src/url.rs`, change `fn open(url: &str) {` to:

```rust
pub(crate) fn open(url: &str) {
```

- [ ] **Step 3: Add the grab flow.** Append to `src/grab.rs`:

```rust
use anyhow::Result;
use crate::picker::{self, Hit};

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
```

- [ ] **Step 4: Add the `Grab` subcommand.** In `src/cli.rs`, before `Menu {`:

```rust
    /// Extract tokens from the pane → copy/paste/open (extrakto-style)
    Grab {
        /// Capture this pane id, then open the picker in a popup (for keybindings)
        #[arg(long)]
        pane: Option<String>,
        /// File with captured pane text — used inside the popup
        source: Option<String>,
    },
```

In `src/main.rs`, add to the match:

```rust
        Cmd::Grab { pane, source } => grab::run(pane.as_deref(), source.as_deref()),
```

- [ ] **Step 5: Build, test, clippy.**

Run: `cargo build && cargo test && cargo clippy --all-targets -- -D warnings`
Expected: builds; tests pass; no warnings.

- [ ] **Step 6: Manual smoke (extractors over a real capture).**

```bash
printf 'go https://github.com/kenanpelit/tmux-anka and /etc/hosts then word\n' \
  | tee /tmp/grabtest >/dev/null
cargo run --release -- grab /tmp/grabtest --pane '' 2>&1 | head   # picker needs a tty; just confirm it builds/launches
```
Expected: with a tty it shows the picker; otherwise it exits cleanly (no panic).

- [ ] **Step 7: Commit.**

```bash
git add src/grab.rs src/clip.rs src/url.rs src/cli.rs src/main.rs
git commit -m "feat(grab): popup flow + Grab subcommand (copy/paste/open)"
```

---

### Task 5: `anka search` (scrollback → jump)

**Files:**
- Create: `src/search.rs`
- Modify: `src/main.rs` (`mod search;` + dispatch), `src/cli.rs` (`Search`)

**Interfaces:**
- Consumes: `picker::pick`, `crate::tmux`.
- Produces: `pub fn goto_line_target(total: usize, top_index: usize) -> usize`; `pub fn run(pane: Option<&str>, source: Option<&str>) -> Result<()>`.

- [ ] **Step 1: Write `goto_line_target` + failing test.** Create `src/search.rs`:

```rust
//! `anka search` — fuzzy-search the scrollback, jump to the line in copy-mode
//! (replaces tmux-fuzzback). Line precision; column precision is out of scope.

use anyhow::Result;

use crate::picker;

/// copy-mode `goto-line` target for a line at `top_index` (0 = oldest captured
/// line). `capture-pane -S -` and copy-mode share the same numbering, so the
/// index *is* the target; clamped to the last line.
pub fn goto_line_target(total: usize, top_index: usize) -> usize {
    top_index.min(total.saturating_sub(1))
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
```

- [ ] **Step 2: Run test to verify it passes (and module compiles after wiring).** Add `mod search;` to `src/main.rs`.

Run: `cargo test search::`
Expected: PASS.

- [ ] **Step 3: Add the flow.** Append to `src/search.rs`:

```rust
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
    let text = crate::tmux::run(&["capture-pane", "-p", "-J", "-t", pane, "-S", "-"])
        .unwrap_or_default();
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
```

- [ ] **Step 4: Add the `Search` subcommand.** In `src/cli.rs`, after `Grab { … }`:

```rust
    /// Fuzzy-search the scrollback → jump to the line (fuzzback-style)
    Search {
        /// Capture this pane id, then open the picker in a popup (for keybindings)
        #[arg(long)]
        pane: Option<String>,
        /// File with captured scrollback — used inside the popup
        source: Option<String>,
    },
```

In `src/main.rs`:

```rust
        Cmd::Search { pane, source } => search::run(pane.as_deref(), source.as_deref()),
```

- [ ] **Step 5: Build, test, clippy.**

Run: `cargo build && cargo test && cargo clippy --all-targets -- -D warnings`
Expected: builds; tests pass; no warnings.

- [ ] **Step 6: Manual smoke (throwaway server, goto-line lands).**

```bash
tmux -L ankas new-session -d -s S -x 80 -y 24
for i in $(seq 1 50); do tmux -L ankas send-keys -t S "echo line-$i" Enter; done
tmux -L ankas capture-pane -p -t S -S - | tail -3   # confirm scrollback present
tmux -L ankas kill-server
```
Expected: scrollback lines printed — confirms capture works for search.

- [ ] **Step 7: Commit.**

```bash
git add src/search.rs src/cli.rs src/main.rs
git commit -m "feat(search): scrollback fuzzy-search → copy-mode jump"
```

---

### Task 6: Bind, retire extrakto + fuzzback, ship 0.12.0

**Files:**
- Modify: `anka.tmux`
- Modify (in `.cachy`): `modules/tmux/dotfiles/tmux/tmux.conf`, remove `extrakto` + `tmux-fuzzback` submodules
- Modify: `Cargo.toml` (version 0.12.0)

- [ ] **Step 1: Bind in `anka.tmux`.** Add the keys near the other `opt`/`bind_anka` lines:

```bash
GRAB_KEY="$(opt @anka-grab-key)";     GRAB_KEY="${GRAB_KEY:-Space}"
SEARCH_KEY="$(opt @anka-search-key)"; SEARCH_KEY="${SEARCH_KEY:-m}"
```

and with the binds:

```bash
bind_anka "$GRAB_KEY"   run-shell -b "$BINARY grab --pane '#{pane_id}'"
bind_anka "$SEARCH_KEY" run-shell -b "$BINARY search --pane '#{pane_id}'"
```

- [ ] **Step 2: Version bump + build + test.**

```bash
sed -i 's/^version = .*/version = "0.12.0"/' Cargo.toml
cargo build --release && ./target/release/anka --version   # anka 0.12.0
cargo test && cargo clippy --all-targets -- -D warnings
```
Expected: `anka 0.12.0`; tests pass; no warnings.

- [ ] **Step 3: Commit + tag + push (canonical).**

```bash
git add anka.tmux Cargo.toml Cargo.lock
git commit -m "feat: bind anka grab (prefix+Space) + search (prefix+m); v0.12.0"
git tag -a v0.12.0 -m "anka v0.12.0 — grab + search (extrakto + fuzzback replacement)"
git push origin HEAD && git push origin v0.12.0
```

- [ ] **Step 4: Update `.cachy` gitlink + binary.**

```bash
cd ~/.cachy/modules/tmux/dotfiles/tmux/plugins/tmux-anka
git fetch origin --tags --quiet && git checkout -q v0.12.0
pkill -x anka 2>/dev/null; sleep 0.5; rm -f bin/anka
cp ~/.kod/tmux/tmux-anka/target/release/anka bin/anka
./bin/anka --version   # anka 0.12.0
```

- [ ] **Step 5: Retire extrakto + fuzzback in `tmux.conf`.** In `~/.cachy/modules/tmux/dotfiles/tmux/tmux.conf`:
  - remove `set -g @plugin 'laktak/extrakto' …` and `set -g @plugin 'roosta/tmux-fuzzback' …`;
  - remove the `@extrakto_*` settings and the `@fuzzback-*` settings;
  - keep `@anka-url-browser` (grab's open reuses it).

- [ ] **Step 6: Remove both submodules + commit/push.**

```bash
cd ~/.cachy
git rm modules/tmux/dotfiles/tmux/plugins/extrakto modules/tmux/dotfiles/tmux/plugins/tmux-fuzzback
rm -rf modules/tmux/dotfiles/tmux/plugins/extrakto modules/tmux/dotfiles/tmux/plugins/tmux-fuzzback
git add modules/tmux/dotfiles/tmux/plugins/tmux-anka modules/tmux/dotfiles/tmux/tmux.conf .gitmodules
git commit -m "tmux: anka v0.12.0 grab+search replace extrakto + tmux-fuzzback"
git push origin HEAD
```

- [ ] **Step 7: Live-apply + verify.**

```bash
tmux source-file ~/.config/tmux/tmux.conf
setsid -f ~/.config/tmux/plugins/tmux-anka/bin/anka daemon
tmux list-keys | grep -E 'anka (grab|search)'   # both binds present
```
Then press `prefix+Space` (grab: token picker, Tab cycles filter, Enter copies) and
`prefix+m` (search: pick a scrollback line → lands in copy-mode there). Confirms
end-to-end. Plugin set is now **3** (tpm, anka, huma).

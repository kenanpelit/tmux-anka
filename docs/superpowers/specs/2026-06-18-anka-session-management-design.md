# anka — native session management (switcher + sessionist) design

Status: approved 2026-06-18. Drives implementation for **v0.8.0**.

## Goal

Bring the functionality of `tmux-sessionist` (quick session actions) and
`tmux-sessionx` (a fuzzy session switcher with preview) **into anka natively**,
so those two plugins can be removed. Everything stays inside the single static
`anka` binary with **zero runtime dependencies** (no fzf, no zoxide *required*),
consistent with the rest of the project. Linux-only, like the rest of anka.

## Decisions (from brainstorming)

- **Replace, don't coexist** — remove `tmux-sessionist` and `tmux-sessionx`;
  anka registers all the keybindings itself (via `anka.tmux`).
- **Switcher UX** — a full interactive TUI rendered by anka itself (raw mode via
  `stty`, ANSI drawing), **not** a TUI crate and **not** delegated to tmux's
  `choose-tree`. Dependency-free.
- **Switcher extras** — all of: inline new/rename/delete, zoxide integration,
  show snapshot (offline) sessions alongside live ones, and window-level entries.
- **Switcher key** — `prefix + s` (overrides the built-in `choose-tree`, which
  anka's switcher supersedes).

## Non-goals

- Replacing `sesh` (a separate external binary the user binds to `Tab`/`C-t`);
  out of scope, left untouched.
- macOS/BSD; mouse support; multi-select.

## Architecture

The risky part is the interactive TUI. Split it into a **pure state machine**
plus a **thin I/O shell**, so all logic is unit-testable without a terminal:

```
src/
  switcher/
    state.rs   PURE: Item, Mode, Key, the model + apply(Key)->Step, fuzzy match.
               No tmux, no I/O — exhaustively unit-tested.
    term.rs    raw mode (stty raw -echo / restore), `stty size`, ANSI helpers,
               stdin byte -> Key parsing (incl. ESC-sequence arrow keys).
    mod.rs     the loop: read Key -> state.apply -> perform tmux/anka effect ->
               refresh preview -> redraw. Falls back to a numbered menu when
               stdin/stdout is not a tty.
  session.rs   sessionist quick actions (new/kill/promote/switch/last/rename),
               thin tmux logic, integration-tested.
```

`tui.rs` (the v0.7.0 numbered picker) becomes the **non-tty fallback** used by
the switcher, so existing pipe-driven tests keep working.

### State machine (`switcher/state.rs`)

```rust
enum Mode { Sessions, Windows, Zoxide }   // Tab cycles; missing zoxide is skipped

enum Item {
    Live(String),                          // a running session
    Snapshot(String),                      // in `last` snapshot, not currently live
    Window { session: String, index: u32, name: String },
    Zoxide(PathBuf),                       // a frecent dir -> new session there
}

enum Key { Up, Down, Enter, Tab, Backspace, Char(char),
           New, Rename, Delete, Cancel }   // New=^n, Rename=^r, Delete=^x, Cancel=Esc/^c

struct State {
    mode: Mode,
    all: Vec<Item>,        // items for the current mode
    filtered: Vec<usize>,  // indices into `all` after fuzzy filter
    query: String,
    cursor: usize,         // index into `filtered`
    prompt: Option<Prompt>,// Some while typing a name for New/Rename
}

// What the I/O layer must do after a key. "Stay" effects keep the TUI open and
// reload items; "Exit" effects close the popup.
enum Step {
    Redraw,                       // query/cursor/mode changed
    PreviewChanged,               // selection moved -> refresh preview pane
    Stay(StayEffect),             // KillSession | RemoveSnapshot | Rename{from,to}
    Exit(ExitEffect),             // SwitchTo | RestoreThenSwitch | SwitchWindow | NewSession | NewInDir | Cancel
}
```

`apply(&mut self, Key) -> Step` is pure (mutates `self`, returns the effect).
Fuzzy matching is a pure subsequence scorer in this module. **Note:** creating a
brand-new session (`^n`) attaches to it, so `NewSession` is an `Exit` effect;
only `Rename`/`Delete` are `Stay` effects that keep the TUI open and reload.
Navigation is via the arrow keys (`Up`/`Down`); `^n/^r/^x` are reserved for the
new/rename/delete actions shown in the switcher footer.

### Switcher behavior

- **Items by mode**
  - *Sessions*: live sessions (label `(live)`) + snapshot-only sessions
    (`(snapshot)`), de-duplicated by name (a live session hides its snapshot
    twin). Enter: live → `switch-client`; snapshot → `restore_one` then switch.
  - *Windows*: every window of every live session; Enter → `switch-client` +
    `select-window`.
  - *Zoxide*: `zoxide query -l` dirs (frecency order); Enter → new session in
    that dir, named after its basename, then switch. Missing `zoxide` (or
    `@anka-zoxide off`) → mode shows `(zoxide unavailable)` and Tab skips it.
- **Filter**: typing appends to `query`; the list is fuzzy-filtered live.
- **Inline actions**: `^n` new (opens a name prompt, prefilled with the query),
  `^r` rename highlighted session (prompt prefilled with its name), `^x` delete
  (live → `kill-session`; snapshot → `anka rm`-equivalent for that session entry
  is out of scope, so `^x` on a snapshot item removes it from the in-memory list
  only and is a no-op on disk — documented). Rename/Delete keep the TUI open and
  reload; New/Enter exit.
- **Preview pane** (`@anka-switch-preview on`, default): live/window →
  `tmux capture-pane -ep` of the target's active pane (last N lines); snapshot →
  its saved pane contents if present, else a structural summary (windows/panes +
  first pane cwd); zoxide → the dir path + a short `read_dir` listing.
- **Fallback**: when not a tty, render the v0.7.0 numbered menu over the same
  *Sessions*-mode items and read one line (used by tests and pipes).
- **Terminal too small**: drop the preview column, list only.

### sessionist quick actions (`session.rs`, not the TUI)

`anka session <action>`:

- `new <name>` — `new-session -d -s <name>` in the active pane's cwd, then
  `switch-client`; if it already exists, just switch.
- `kill` — switch the client to another session, then `kill-session` the
  previous one (never kills the server out from under the last session: if it's
  the only one, refuse with a message).
- `promote <name>` — move the current pane into a new session: capture the
  current pane id and cwd, `new-session -d -s <name> -c <cwd>`, `move-pane -s
  <pane> -t <name>`, `kill-pane` the placeholder shell, `select-layout tiled`,
  then `switch-client`.
- `switch <name>` — `switch-client -t <name>` (error if absent).
- `last` — `switch-client -l`.
- `rename <new>` — `rename-session <new>` (also invoked by the TUI's `^r`).

Name-prompt actions are wired with tmux's native `command-prompt` in `anka.tmux`.

## CLI surface (additions)

```
anka switch                Interactive session switcher (live + snapshot + zoxide)
anka session new <name>    Create/switch to a named session
anka session kill          Kill the current session, switching away first
anka session promote <name>  Move the current pane into a new session
anka session switch <name> Switch to a session by name
anka session last          Switch to the last session
anka session rename <new>  Rename the current session
```

`anka pick` (v0.7.0) is redirected to `anka switch` — the switcher already lists
snapshot sessions, so the old numbered picker becomes its non-tty fallback.

## Configuration (additions; all optional, defaulted)

| Option | Default | Meaning |
|--------|---------|---------|
| `@anka-switch-key` | `s` | Open the switcher |
| `@anka-new-key` | `C` | New named session |
| `@anka-kill-key` | `X` | Kill current session |
| `@anka-promote-key` | `@` | Promote pane to a new session |
| `@anka-switch-name-key` | `g` | Switch to session by name |
| `@anka-last-key` | `S` | Switch to last session |
| `@anka-switch-preview` | `on` | Show the preview pane in the switcher |
| `@anka-zoxide` | `on` | Enable the zoxide mode when `zoxide` is present |

## Keybindings (registered by `anka.tmux`)

```
prefix + s   display-popup -E "anka switch"
prefix + C   command-prompt -p "New session:" → anka session new "%%"
prefix + X   anka session kill
prefix + @   command-prompt -p "Promote pane to session:" → anka session promote "%%"
prefix + g   command-prompt -p "Switch to session:" → anka session switch "%%"
prefix + S   anka session last
```
(plus the existing `C-s`/`C-r` save/restore and `P` pick → switch.)

## Migration (remove the replaced plugins)

In `~/.cachy` (the dotfiles repo):

- Remove the `tmux-plugins/tmux-sessionist` and `omerxx/tmux-sessionx` lines
  from `tmux.conf`, plus any `@sessionx-*` / `@sessionist-*` settings (grep
  first; migrate any custom key to the matching `@anka-*-key`).
- Remove the two gitlinks + working dirs (as done for `tmux-update-display`),
  commit, and bump the anka gitlink.
- `sesh` (Tab / C-t) is left as-is.

## Error handling

- No tmux server → message, exit non-zero.
- No sessions and no snapshot → switcher shows an empty list with `^n` to create.
- `zoxide` absent → zoxide mode unavailable (not an error).
- Always restore the terminal (`stty sane`) on normal exit; the popup teardown
  by tmux is the backstop if anka aborts in raw mode (`panic = abort` means Drop
  guards do not run, so we must not rely on them for correctness — the popup pty
  is discarded by tmux regardless).

## Testing

- **Unit (pure):** fuzzy matcher; `State::apply` over scripted key sequences
  (query edits, navigation, mode cycling, prompt flows) asserting query/cursor/
  mode and the returned `Step`; the stdin-byte → `Key` parser incl. arrow ESC
  sequences; item assembly (live+snapshot merge/de-dupe); `zoxide query -l`
  output parsing.
- **Integration (Server harness):** `anka session new/kill/promote/switch/last/
  rename` against a throwaway server; the switcher's non-tty fallback (piped
  stdin) switching to a chosen live session and restoring a chosen snapshot one.

## Rollout

Ships as **v0.8.0** (tagged → CI publishes binaries). README + DESIGN updated:
new Session-management section, keybinding table, config table, and the picker
note. Roadmap gains a v0.8.0 entry.

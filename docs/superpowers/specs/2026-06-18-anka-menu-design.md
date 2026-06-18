# anka menu — native tmux action menu (tmux-fzf port)

Status: approved 2026-06-18.

## Goal

Replace the `sainnhe/tmux-fzf` plugin with a native `anka menu` subcommand: a
`prefix+F` action menu, built on anka's own raw-mode picker (no `fzf` runtime
dependency), covering the categories the anka switcher doesn't already provide.
Retire `tmux-fzf` afterwards.

## Scope

Five categories (session switching is already covered by the anka switcher,
`prefix+o`, so it is intentionally **not** duplicated here):

- **command** — run a tmux command
- **keybinding** — browse and execute a key binding
- **process** — kill a system process
- **window** — manage a window (switch / kill / rename)
- **pane** — manage a pane (switch / kill / zoom / break)

**Out of scope (YAGNI):** session category (switcher covers it), copy-mode entry,
multi-select, window swap/move/link, pane resize beyond zoom, fzf preview panes.
These can be added later if wanted.

## Entry point & flow

A new `anka menu` subcommand, bound to `prefix+F` (key configurable via
`@anka-menu-key`, default `F`). Two-step launch, mirroring the proven `anka url`
pattern (because `#{...}` formats expand in a keybinding's `run-shell` command but
**not** inside `display-popup -E`):

1. **Capture step (no tty):** the bind runs
   `run-shell -b "<bin>/anka menu --client '#{client_name}' --session '#{session_name}'"`.
   tmux expands the formats, so anka receives the real invoking client/session.
   anka then opens the interactive picker inside a popup:
   `display-popup -E "<bin>/anka menu --run --client <c> --session <s>"`.
2. **Picker step (in the popup tty):** anka runs the category → target → action
   flow and applies the action, targeting the **invoking** client/session (not the
   popup's own client) via `tmux -t <session>:<window>` / `switch-client -c <client>`.

Flow depth per category:

```
prefix+F → [category]   command · keybinding · process · window · pane
              │
   command/keybinding/process: pick target → apply immediately   (2 levels)
   window/pane:                pick target → [action menu]        (3 levels)
```

Every level uses the same native picker (switcher/url look): fuzzy filter, arrow
/ Ctrl-n/Ctrl-p navigation, 1–9 quick-jump, Esc cancels (backs up one level).

## Category behavior

| Category | Source | Action |
|----------|--------|--------|
| command | `tmux list-commands` | pick → `command-prompt -I "<cmd> "` on the invoking client (user fills args & runs) |
| keybinding | `tmux list-keys` | pick → run that binding's command on the invoking client |
| process | `ps -eo pid,comm,args` (or similar) | pick → `kill` (SIGTERM) the pid |
| window | `tmux list-windows -a` | pick → action menu: switch / kill / rename |
| pane | `tmux list-panes -a` (incl. command + path) | pick → action menu: switch / kill / zoom / break |

Window/pane action commands:

- **switch**: `select-window -t <session>:<index>` (and `select-pane -t` for pane);
  if the target session differs from the invoking one, also `switch-client -c <client> -t <session>`.
- **kill**: `kill-window` / `kill-pane -t …`.
- **rename** (window): `command-prompt -I "<name>" "rename-window -t <id> '%%'"`.
- **zoom** (pane): `resize-pane -Z -t …`.
- **break** (pane): `break-pane -s …`.

`process` kill sends SIGTERM (not SIGKILL). Selection in the picker is the
deliberate confirm; no extra prompt.

## Components

- **`src/picker.rs`** (new, shared): `pick(items: &[String], prompt: &str) ->
  Result<Option<usize>>` — a single-column fuzzy picker reusing
  `switcher::term` (`RawMode`, `parse_key`, `term_size`, `move_to`),
  `switcher::fuzzy_score`, and `switcher::Key`. `url.rs::pick_interactive` is
  refactored onto it (removes the duplicate loop). The switcher's specialised
  two-pane/preview loop stays as-is for now.
- **`src/menu.rs`** (new): the menu logic.
  - Pure, unit-tested: parsing source-command output into item rows
    (`parse_processes`, `parse_windows`, `parse_keys`, …) and the
    action → tmux-argv builders (e.g. `switch_window_argv(session, index)`).
  - I/O glue: run the source commands, drive the picker, dispatch the chosen
    tmux command. Targets the invoking client/session passed in from the capture
    step.
- **`src/cli.rs` / `src/main.rs`**: a `Menu { run: bool, client: Option<String>,
  session: Option<String> }` subcommand. `--run` distinguishes the popup picker
  step from the capture step.
- **`anka.tmux`**: bind `@anka-menu-key` (default `F`) to the capture-step
  `run-shell`. Honour `@anka-menu-key 'none'` to skip binding (consistent with the
  other anka keys).
- **`src/config.rs`**: read `@anka-menu-key`.

## Error handling

- Empty source (no windows/panes/processes) → a `display-message` ("anka: nothing
  to …") and exit, like `anka url`'s "no URLs" path.
- Cancel (Esc) at the action menu backs up to the target list; Esc at the top
  cancels the whole menu.
- A failed tmux command surfaces via `display-message`; anka never panics on a
  missing target (it may have closed meanwhile).

## Testing

- **Unit:** the pure parsers (`ps`/`list-windows`/`list-panes`/`list-keys` text →
  item rows) and the action → argv builders. Mirror anka's existing
  table-style tests (e.g. `url.rs`, `daemon.rs`).
- **Integration (throwaway tmux server):** `anka menu` against a known session —
  assert the built switch/kill argv targets the right `session:window`. The
  raw-mode picker itself is exercised manually (as with the switcher/url picker).

## Migration (retire tmux-fzf)

1. `anka.tmux`: add the `prefix+F` → `anka menu` bind.
2. `tmux.conf`: remove the `tmux-fzf` `@plugin` line and the dead `@fzf-*`
   settings; keep `prefix+F` working via anka.
3. Remove the `tmux-fzf` submodule/gitlink from `.cachy`.
4. Bump anka to **0.10.0** (new feature), tag/push; bump the `.cachy` gitlink +
   binary; live-apply.

This drops the tmux plugin count to 6 (tpm, anka, huma, yank, fuzzback, extrakto).

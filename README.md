# tmux-anka 🔥

> Freeze and resurrect tmux sessions — *exactly*.

**Anka** (the phoenix that rises from its ashes, unchanged) is a single modern
plugin that does what `tmux-resurrect` **and** `tmux-continuum` do together —
rewritten from scratch in Rust as one zero-runtime-dependency static binary,
with a clean JSON snapshot format.

It saves your whole tmux environment (sessions, windows, panes, layouts, working
directories, pane contents, running programs) and brings it back identical after
a restart — automatically.

## Features

- **Exact restore** — every session/window/pane, the precise tmux layout, and
  each pane's working directory. Scrollback contents are captured into the
  snapshot too (optional), but replaying them into restored panes is not
  implemented yet — see the roadmap.
- **JSON snapshots** — human-readable, hand-editable, one directory per snapshot.
- **Event-driven auto-save** — saves on client detach via a native hook, plus
  an optional interval daemon for periodic saves. (Session/window close is
  deliberately *not* hooked: it fires after the session is gone, so saving
  there would prune it from the snapshot.) No status-bar piggybacking.
- **Auto-restore on start** — your last snapshot comes back when the tmux server
  starts.
- **Named snapshots** — `anka save work`, `anka restore work`, `anka list`.
- **Lazy / per-session restore** — built-in dependency-free picker
  (`prefix + P`) restores just the session you choose, saving memory.
- **Program restore** — relaunches allow-listed programs, faithfully preserving
  their arguments (incl. repairing the `--` that `npm exec`/`npx` drop from
  their process title).
- **nvim/vim sessions** — resumes a `Session.vim` when present (the `session`
  strategy), otherwise reopens the same files.
- **Freeze to blueprint** — turn a snapshot into a re-runnable declarative spec
  (`anka up <name>`) or an exportable standalone shell script.
- **Built-in session manager** — an interactive switcher (`prefix + o`) over
  live + snapshot + zoxide sessions, with a bordered two-pane layout, a live
  colour preview, numbered jump, fuzzy filter, and inline
  new/rename/kill, plus sessionist-style quick actions (new/kill/promote/switch/
  last). Replaces `tmux-sessionx` + `tmux-sessionist`; no external session
  manager or fuzzy-finder needed.

## Requirements

- **Linux** — process resolution reads `/proc` (no macOS/BSD support).
- **tmux 3.x+** and a working `tmux` on `PATH`.
- Nothing else at runtime: `anka` is a single static binary. Building from
  source needs a Rust toolchain; installing a release does not.

## Install (TPM)

Add to your `tmux.conf`:

```tmux
set -g @plugin 'kenanpelit/tmux-anka'
```

Then hit `prefix + I`. On first load the plugin resolves the binary **inside the
plugin directory** (never touching your `PATH`): it downloads the prebuilt
release asset for your architecture (`x86_64` / `aarch64`), or compiles it with
`cargo` if no asset matches and Rust is available.

### Manual install

```sh
git clone https://github.com/kenanpelit/tmux-anka \
    ~/.tmux/plugins/tmux-anka
~/.tmux/plugins/tmux-anka/scripts/install-binary.sh   # fetch or build the binary
# then `run ~/.tmux/plugins/tmux-anka/anka.tmux` from your tmux.conf
```

## Keybindings

| Key | Action |
|-----|--------|
| `prefix + C-s` | Save snapshot |
| `prefix + C-r` | Restore last snapshot |
| `prefix + o` | Session switcher (live + snapshot + zoxide) |
| `prefix + P` | Pick a session to restore (opens the switcher) |
| `prefix + C` | New named session |
| `prefix + X` | Kill the current session |
| `prefix + @` | Promote the current pane to a new session |
| `prefix + g` | Switch to a session by name |
| `prefix + S` | Switch to the last session |
| `prefix + F` | Action menu (sessions/windows/panes/snapshots) |
| `prefix + Space` | Grab text from the screen (extrakto-style) |
| `prefix + m` | Search scrollback and jump (fuzzback-style) |

Three defaults shadow tmux builtins: `Space` (next-layout), `m` (mark pane)
and `o` (next pane). Set any `@anka-*-key` option to `none` to skip that
binding and keep the builtin (or your own).

## Usage

Day to day you do nothing: anka auto-saves when you detach and (optionally) on
an interval, and auto-restores your `last` snapshot when the tmux server
starts. Session/window close is intentionally not hooked — that event fires
after the session is already gone, so a save there would drop it from the
snapshot (and cascade to an empty one on logout). The rest is on demand.

**Named snapshots** — keep curated layouts alongside the rolling `last` one:

```sh
anka save work          # snapshot the current environment as "work"
anka list               # default (last), work
anka restore work       # bring "work" back (never clobbers a live session)
anka rm work
```

**Switch sessions** (`prefix + o`) — a bordered popup over your live sessions
*and* the offline ones in your last snapshot (and, with `zoxide`, frecent dirs),
with a live colour preview of the highlighted target. Rows are numbered: press
`1`-`9` to jump straight to one. Type to fuzzy-filter, move with `↑`/`↓` or
`^p`/`^n`, and `Tab` cycles sessions/windows/zoxide:

```
╭─ anka · sessions ──────────╮╭─ preview: dev ───────────────╮
│▌ 1 KENP        live · 3w 5p││ ~/dev/anka                    │
│  2 dev         live · 2w 3p││ $ cargo test                  │
│  3 media       live · 2w 2p││ test result: ok. 19 passed    │
│  4 Tor         snapshot ·2w││ $ ▮                           │
│ › dev▌                     ││                               │
╰────────────────────────────╯╰───────────────────────────────╯
 ↑↓/^p^n move · 1-9 jump · ⏎ go (type+⏎ new) · ^r rename · ^x kill · ⇥ mode · esc
```

The preview is colour (it shows each pane's own output); turn it off with
`@anka-switch-preview off` for a plain bordered list.

**Open a URL from the screen** — `anka url` extracts http(s) URLs from pane text
(trailing punctuation trimmed, de-duplicated), shows them in the same bordered
picker, and opens the chosen one in your browser — `@anka-url-browser` if set,
else `$BROWSER`, else `xdg-open`. No fzf/extra tools. Bind it:

```tmux
bind u run-shell -b "~/.tmux/plugins/tmux-anka/bin/anka url --pane '#{pane_id}' --client '#{client_name}'"
```

`anka url --pane` captures the pane (run-shell expands `#{pane_id}`; a
`display-popup` would not), and if it finds URLs reopens itself in a popup for
the picker. `anka url [file]` / `anka url` (stdin) are the picker itself.

`⏎` switches to a live session, restores a snapshot one, jumps to a window, or
opens a zoxide dir as a new session. Type a name that matches nothing and `⏎` to
create it; `^r` renames the selected session, `^x` kills it. A preview pane
(`@anka-switch-preview on`) shows the highlighted target. (`prefix + P` opens the
same switcher; on a non-tty it degrades to a numbered menu.) For quick actions
there are also `prefix + C` (new), `X` (kill), `@` (promote pane), `g` (switch
by name), and `S` (last session). `C`/`@`/`g` ask for the name with an inline
prompt in a small popup — the typed input goes straight to anka, never through
a shell.

**Freeze a layout to a re-runnable blueprint** — a hand-editable template you can
relaunch anywhere, independent of the rolling snapshots:

```sh
anka freeze work            # → <anka-dir>/blueprints/work.json (edit by hand)
anka up work                # recreate the layout from the blueprint
anka freeze work --script   # also export blueprints/work.sh (raw tmux, no anka)
```

Programs are relaunched into the pane (so it survives if the command exits), with
their arguments preserved — including repairing the `--` separator that
`npm exec`/`npx` drop from their process title, so `npm exec pkg -r --flag` comes
back with `-r --flag` intact.

## Status widget

Show the last-save indicator in your status bar:

```tmux
set -g status-right "… #{@anka_status} …"
```

## Configuration

| Option | Default | Meaning |
|--------|---------|---------|
| `@anka-dir` | `${XDG_DATA_HOME}/tmux/anka` | Snapshot storage directory |
| `@anka-capture-pane-contents` | `on` | Capture pane scrollback |
| `@anka-history-limit` | `2000` | Max scrollback lines captured per pane (`0` = unlimited) |
| `@anka-save-generations` | `3` | Rotated snapshot generations to keep (`0` = off) |
| `@anka-restore-processes` | `ssh psql mysql sqlite3 npm yarn nvim` | Programs to relaunch |
| `@anka-restore-verbatim` | `off` | Relaunch captured commands verbatim, without the allow-list |
| `@anka-strategy-nvim` | `session` | nvim/vim restore strategy |
| `@anka-save-interval` | `10` | Interval daemon period in minutes (`0` disables) |
| `@anka-restore-on-start` | `on` | Auto-restore last snapshot on server start |
| `@anka-restore-overwrite` | `off` | Overwrite existing sessions on restore |
| `@anka-switch-preview` | `on` | Show the preview pane in the switcher |
| `@anka-zoxide` | `on` | Enable the zoxide mode when `zoxide` is installed |
| `@anka-save-key` / `@anka-restore-key` / `@anka-pick-key` | `C-s` / `C-r` / `P` | Snapshot keys |
| `@anka-switch-key` | `o` | Open the switcher (leaves `prefix + s` = choose-tree) |
| `@anka-new-key` / `@anka-kill-key` / `@anka-promote-key` | `C` / `X` / `@` | Session new/kill/promote |
| `@anka-switch-name-key` / `@anka-last-key` | `g` / `S` | Switch by name / last session |
| `@anka-menu-key` | `F` | Open the action menu |
| `@anka-grab-key` / `@anka-search-key` | `Space` / `m` | Grab screen text / search scrollback |
| `@anka-url-browser` | _(unset → `$BROWSER` → `xdg-open`)_ | Command `anka url` opens the chosen URL with |
| `@anka-popup-width` / `@anka-popup-height` | `80%` / `75%` | Popup size for the switcher/pick TUIs (any `display-popup` size, e.g. `90%` or `120`) |

Set any `@anka-*-key` to `none` to skip that binding and keep your own (some
defaults shadow tmux builtins — see [Keybindings](#keybindings)).

## CLI

```
anka save [name]        Save current environment to a snapshot
anka restore [name]     Restore a snapshot (default: last)
anka list               List saved snapshots
anka rm <name>          Remove a snapshot
anka pick               Open the session switcher (alias of `switch`)
anka switch             Interactive session switcher (live + snapshot + zoxide)
anka menu               Action menu: sessions/windows/panes/snapshots (popup)
anka url [file]         Pick a URL from pane text (or stdin) and open in $BROWSER
anka grab --pane <id>   Grab words/paths/URLs from the visible pane text (extrakto-style)
anka search --pane <id> Fuzzy-search scrollback and jump to the match (fuzzback-style)
anka clip               Copy stdin to the system clipboard (OSC 52)
anka session new [name]     Create / switch to a named session (--prompt asks inline)
anka session kill           Kill the current session, switching away first
anka session promote [name] Move the current pane into a new session (--prompt)
anka session switch [name]  Switch to a session by name (--prompt)
anka session last           Switch to the last session
anka session rename <new>   Rename the current session
anka freeze [name]      Freeze a snapshot to a declarative blueprint
anka freeze --script    …also export a standalone shell script
anka up <name>          Re-launch a frozen blueprint
anka status             Print the status-bar widget text
anka daemon             Run the interval auto-save daemon
anka doctor             Check the setup: binary, tmux, options, snapshot health
```

## Design

See [`docs/DESIGN.md`](docs/DESIGN.md) for the full architecture and rationale.

## License

MIT © Kenan Pelit

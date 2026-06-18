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

- **Exact restore** — every session/window/pane, the precise tmux layout, each
  pane's working directory, and (optionally) scrollback contents.
- **JSON snapshots** — human-readable, hand-editable, one directory per snapshot.
- **Event-driven auto-save** — saves on meaningful tmux events (session/window
  close, detach) via native hooks; optional interval daemon for periodic saves.
  No status-bar piggybacking.
- **Auto-restore on start** — your last snapshot comes back when the tmux server
  starts.
- **Named snapshots** — `anka save work`, `anka restore work`, `anka list`.
- **Lazy / per-session restore** — built-in TUI picker (`prefix + C-p`) restores
  just the session you choose, saving memory.
- **Program restore** — relaunches allow-listed programs by opening the pane
  *with* the command (not by typing into a shell).
- **nvim/vim sessions** — restored via the `Session.vim` strategy.
- **Freeze to blueprint** — turn a snapshot into a re-runnable declarative spec
  (`anka up <name>`) or an exportable standalone shell script.

## Install (TPM)

Add to your `tmux.conf`:

```tmux
set -g @plugin 'kenanpelit/tmux-anka'
```

Then hit `prefix + I`. On first load the plugin fetches a prebuilt binary for
your platform (or compiles it with `cargo` if Rust is available) into the
plugin directory — nothing is written to your `PATH`.

## Keybindings

| Key | Action |
|-----|--------|
| `prefix + C-s` | Save snapshot |
| `prefix + C-r` | Restore last snapshot |
| `prefix + C-p` | Pick a session to restore (TUI) |

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
| `@anka-restore-processes` | `ssh psql mysql sqlite3 npm yarn nvim` | Programs to relaunch |
| `@anka-strategy-nvim` | `session` | nvim/vim restore strategy |
| `@anka-save-interval` | `10` | Interval daemon period in minutes (`0` disables) |
| `@anka-restore-on-start` | `on` | Auto-restore last snapshot on server start |
| `@anka-restore-overwrite` | `off` | Overwrite existing sessions on restore |
| `@anka-save-key` / `@anka-restore-key` / `@anka-pick-key` | `C-s` / `C-r` / `C-p` | Keybindings |

## CLI

```
anka save [name]        Save current environment to a snapshot
anka restore [name]     Restore a snapshot (default: last)
anka list               List saved snapshots
anka rm <name>          Remove a snapshot
anka pick               Interactive per-session restore (TUI)
anka freeze [name]      Freeze a snapshot to a declarative blueprint
anka freeze --script    …also export a standalone shell script
anka up <name>          Re-launch a frozen blueprint
anka status             Print the status-bar widget text
anka daemon             Run the interval auto-save daemon
```

## Design

See [`docs/DESIGN.md`](docs/DESIGN.md) for the full architecture and rationale.

## License

MIT © Kenan Pelit

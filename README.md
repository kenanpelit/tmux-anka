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
- **Lazy / per-session restore** — built-in dependency-free picker
  (`prefix + P`) restores just the session you choose, saving memory.
- **Program restore** — relaunches allow-listed programs, faithfully preserving
  their arguments (incl. repairing the `--` that `npm exec`/`npx` drop from
  their process title).
- **nvim/vim sessions** — resumes a `Session.vim` when present (the `session`
  strategy), otherwise reopens the same files.
- **Freeze to blueprint** — turn a snapshot into a re-runnable declarative spec
  (`anka up <name>`) or an exportable standalone shell script.

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
| `prefix + P` | Pick a session to restore |

## Usage

Day to day you do nothing: anka auto-saves on session/window close and detach,
optionally on an interval, and auto-restores your `last` snapshot when the tmux
server starts. The rest is on demand.

**Named snapshots** — keep curated layouts alongside the rolling `last` one:

```sh
anka save work          # snapshot the current environment as "work"
anka list               # default (last), work
anka restore work       # bring "work" back (never clobbers a live session)
anka rm work
```

**Pick one session** (`prefix + P`) — restore just what you need instead of
everything. A numbered menu opens in a popup:

```
anka — restore a session from snapshot 'default':

   1)  KENP                     3 win · 5 panes  (live)
   2)  Tor                      1 win · 2 panes
   3)  media                    2 win · 3 panes

select [1-3], or q to cancel:
```

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
| `@anka-restore-processes` | `ssh psql mysql sqlite3 npm yarn nvim` | Programs to relaunch |
| `@anka-strategy-nvim` | `session` | nvim/vim restore strategy |
| `@anka-save-interval` | `10` | Interval daemon period in minutes (`0` disables) |
| `@anka-restore-on-start` | `on` | Auto-restore last snapshot on server start |
| `@anka-restore-overwrite` | `off` | Overwrite existing sessions on restore |
| `@anka-save-key` / `@anka-restore-key` / `@anka-pick-key` | `C-s` / `C-r` / `P` | Keybindings |

## CLI

```
anka save [name]        Save current environment to a snapshot
anka restore [name]     Restore a snapshot (default: last)
anka list               List saved snapshots
anka rm <name>          Remove a snapshot
anka pick               Interactive per-session restore
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

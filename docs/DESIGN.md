# tmux-anka — Design

Status: approved 2026-06-18. This is the brainstorming spec that drives
implementation.

## Goal

A single, modern, from-scratch plugin that provides everything `tmux-resurrect`
and `tmux-continuum` do together, plus selected ideas from the broader
ecosystem (lazy per-session restore, named snapshots, freeze-to-blueprint).
Written in Rust as one static binary with **zero runtime dependencies**, using a
human-readable **JSON** snapshot format. Linux-only (uses `/proc`).

## Non-goals (v1)

- macOS/BSD support (Linux-only; `/proc`-based process resolution).
- Auto-starting the tmux server on boot — left to the user's systemd unit.

## Distribution

- Source of truth: standalone repo `github.com/kenanpelit/tmux-anka`.
- `cargo-dist`/CI publishes static binaries to GitHub Releases on tag.
- Installed via TPM (`set -g @plugin 'kenanpelit/tmux-anka'`). The `anka.tmux`
  entrypoint resolves the binary **inside the plugin directory** (like
  `tmux-thumbs`), never touching `PATH`. If missing or version-mismatched, it
  downloads the matching release asset, or compiles with `cargo` as a fallback.

## Components

```
anka.tmux            TPM entrypoint (bash): resolve binary, binds, hooks, status
scripts/install-binary.sh   download-release-or-cargo-build
src/
  main.rs / cli.rs   clap subcommand dispatch
  tmux.rs            tmux CLI wrapper (run + query with -F)
  model.rs           serde structs: Snapshot/Session/Window/Pane
  config.rs          read @anka-* via `tmux show-options -gqv`
  store.rs           snapshot dirs, `last` symlink, list/rm
  capture.rs         save: query tmux -> model -> JSON (atomic write)
  process.rs         foreground command resolution via /proc
  restore.rs         restore: JSON -> tmux commands
  nvim.rs            nvim/vim Session.vim strategy
  daemon.rs          interval auto-save daemon (single-instance)
  tui.rs             ratatui per-session picker
  freeze.rs          snapshot -> declarative blueprint + shell script export
  status.rs          #{@anka_status} text
```

## Snapshot storage

```
${XDG_DATA_HOME:-~/.local/share}/tmux/anka/snapshots/
├── last -> default/          # symlink to most recent
├── default/
│   ├── snapshot.json
│   └── panes/                # pane contents (when enabled)
│       └── KENP@1.1.txt
└── work/  ...                # named snapshots, one dir each
```

## JSON schema

```json
{
  "schema": 1,
  "anka_version": "0.1.0",
  "saved_at": "2026-06-18T11:30:00+03:00",
  "client": { "active_session": "KENP", "last_session": "Tor" },
  "sessions": [{
    "name": "KENP",
    "windows": [{
      "index": 1, "name": "npm", "active": true,
      "layout": "b8df,210x53,0,0,2", "automatic_rename": true,
      "panes": [{
        "index": 1, "active": true,
        "title": "…", "cwd": "/home/kenan/.cachy",
        "command": "npm", "pid": 12345, "history_size": 1234,
        "contents": "panes/KENP@1.1.txt",
        "restore": { "kind": "process", "command": "npm exec …" }
      }]
    }]
  }]
}
```

`restore.kind` ∈ `shell | process | nvim`, resolved at **capture** time so
restore is deterministic.

## Capture

- One `tmux list-panes -a -F` and one `tmux list-windows -a -F` call, fields
  separated by US (`\x1f`) to avoid the tab/newline injection bugs that affect
  resurrect's TSV parsing.
- Per-pane foreground command resolved from `/proc/<pane_pid>` (walk children,
  skip shells) — drives `restore.kind`/`command`.
- Pane contents captured with `tmux capture-pane -p` into `panes/` when
  `@anka-capture-pane-contents on`.
- Atomic write: `snapshot.json.tmp` → rename, then update `last` symlink.

## Restore

Deterministic order:
1. Create each session (`new-session -d -s <name> -c <cwd>`); skip existing
   unless `@anka-restore-overwrite on`.
2. Create windows (`new-window -c <cwd>`), set names.
3. Create panes (`split-window -c <cwd>`), then `select-layout <layout>` for
   pixel-exact geometry.
4. Restore pane contents (when present).
5. Relaunch programs by `restore.kind`: `shell` → nothing; `process` →
   open pane with the command + `remain-on-exit` (fallback: `send-keys`);
   `nvim` → `nvim -S <Session.vim>`.
6. Set active window/pane, then client `active`/`last` session.

Non-destructive by default; partial restore continues best-effort with a
summary. Linux-only `/proc` keeps process logic simple.

## Auto-save (event-driven + optional daemon)

- `anka.tmux` registers native tmux hooks (`session-closed`, `client-detached`,
  structural window changes) → `anka hook <event>` → debounced `anka save`.
  No polling, no status-interval piggyback.
- Optional `anka daemon` provides periodic interval saves
  (`@anka-save-interval` minutes); single-instance via a lockfile; exits when
  the tmux server is gone.

## Auto-restore

On plugin load, if `@anka-restore-on-start on` and no per-server sentinel
exists yet, run `anka restore` (the `last` snapshot). Once per server start.

## Status widget

`anka status` prints e.g. `✔ 11:30` (or `⟳` while saving). The binary updates
the `@anka_status` user option when it saves; users reference `#{@anka_status}`.

## Picker (lazy restore)

`anka pick` is a built-in ratatui fuzzy picker (zero external deps) listing the
sessions in a snapshot; the chosen session is restored on its own. Bound to a
tmux `display-popup`.

## Freeze

`anka freeze [name]` writes a declarative, hand-editable blueprint that
`anka up <name>` re-launches. `anka freeze --script` additionally exports a
self-contained POSIX shell script that recreates the layout without `anka`.

## Configuration

See the table in `README.md`. All options read via `tmux show-options -gqv`.

## Error handling

- Missing/stale binary → install flow with clear messaging.
- No tmux server → no-op with message.
- Corrupt/missing snapshot → fail without destroying current state; log to
  `<dir>/anka.log`.
- All writes atomic (tmp + rename); restore non-destructive by default.

## Testing

- Unit: format-string → model parsing; model → tmux command sequence (golden);
  JSON round-trip.
- Integration: spin a throwaway server (`tmux -L anka-test`), build a known
  tree, `save` → `kill-server` → `restore`, assert via `list-panes -F`.

## Roadmap

- **v0.1.0** — scaffold, config/store/model/tmux infra, real `anka save`
  (capture → JSON, incl. pane contents + process resolution). Restore/daemon/
  tui/freeze stubbed.
- **v0.2.0** — `anka restore` (full deterministic rebuild) + integration tests.
- **v0.3.0** — event-driven auto-save hooks + auto-restore + status widget.
- **v0.4.0** — interval daemon, named snapshots polish.
- **v0.5.0** — ratatui picker (lazy per-session restore).
- **v0.6.0** — freeze (blueprint + shell export) and `anka up`.
- **v1.0.0** — nvim strategy, docs, release binaries via CI.

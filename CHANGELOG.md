# Changelog

All notable changes to tmux-anka are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [0.13.1] - 2026-07-04

### Fixed

- Snapshot capture (and the menu's window/pane lists) parsed as empty on
  tmux <= 3.4, which octal-escapes the US (0x1f) field separator in `-F`
  output as the literal text `\037`; tmux >= 3.5 prints the raw byte.
  Both forms are now accepted, so `save`/`freeze`/`up`/`menu` work across
  versions.

## [0.13.0] - 2026-07-04

### Added

- `anka doctor` — checks the setup: binary, tmux, options, snapshot health.
- `--prompt` on `anka session new|promote|switch`: an inline raw-mode name
  prompt inside a small popup (this is how the default keybindings collect
  names now).
- `--client` on `anka url|grab|search` so the picker popup opens on the
  invoking client.
- New options: `@anka-history-limit` (max captured scrollback lines per pane;
  default `2000`, `0` = unlimited), `@anka-save-generations` (rotated snapshot
  generations to keep; default `3`, `0` = off), `@anka-restore-verbatim`
  (default `off`).
- CI: a `test` workflow — `cargo clippy --all-targets -- -D warnings`,
  `cargo test` (with tmux installed), `shellcheck anka.tmux scripts/*.sh`.
- Release: a `SHA256SUMS` asset covering both release tarballs;
  `install-binary.sh` verifies it before installing.

### Changed

- Release binaries are genuinely static musl builds now
  (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`); asset file
  names are unchanged (`anka-<ver>-<arch>-linux.tar.gz`).
- `install-binary.sh` downloads into a private `mktemp -d` directory (no more
  fixed `/tmp` path), verifies the checksum, and replaces `bin/anka` with an
  atomic rename — safe while the interval daemon is running the old binary.
- Keybindings simplified to plain `run-shell -b`: inside tmux the binary
  reports results via `display-message` itself and keeps stdout quiet, so
  nothing lands in a press-q-to-dismiss output view anymore.
- The menu binding no longer passes `--session`.
- `Cargo.toml` declares `rust-version = "1.85"`.

### Fixed

- The auto-save hook uses an indexed slot (`client-detached[27]`) so it
  coexists with user/plugin `client-detached` hooks instead of overwriting
  them; the plugin also no longer unsets the global `session-closed` hook
  (it may be *yours*). If you upgraded from anka <0.9.4 and a stale
  anka-owned hook lingers, run `tmux set-hook -gu session-closed` once.
- `$BINARY` is quoted in every binding, so a plugin path containing spaces
  works.

### Security

- Shell injection through `command-prompt` `'%%'` interpolation in the
  session new/promote/switch bindings, and through `#{session_name}`
  interpolation in the menu binding: typed names and session names no longer
  pass through a shell at all (inline `--prompt` popups; `--session` dropped).

## Earlier releases (pre-changelog, from git tags)

- v0.12.7 — robust URL char-class extraction
- v0.12.6 — @plugin-only GitHub shorthand
- v0.12.5 — prominent selected row in the picker
- v0.12.4 — live fuzzy match highlight in the picker
- v0.12.3 — search via copy-mode search (exact + highlight)
- v0.12.2 — picker bottom-anchored (fzf-style)
- v0.12.1 — search: faithful fuzzback jump
- v0.12.0 — grab + search (extrakto + fuzzback replacement)
- v0.11.0 — anka clip (tmux-yank replacement)
- v0.10.1 — url: GitHub owner/repo shorthand
- v0.10.0 — native action menu (tmux-fzf replacement)
- v0.9.5 — url: www. + scheme-less domains
- v0.9.4 — fix session-closed snapshot prune
- v0.9.3 — anka url --pane (one-line keybinding)
- v0.9.2 — @anka-url-browser
- v0.9.1 — anka url opens reliably (wait for setsid)
- v0.9.0 — anka url (native URL picker)
- v0.8.2 — choose-tree-class switcher (bordered + colour preview)
- v0.8.1 — switcher numbered jump + ^p/^n nav + prefix+o
- v0.8.0 — native session switcher + sessionist actions
- v0.7.0 — picker, freeze/up, nvim strategy, npm -- repair
- v0.1.0–v0.6.0 — untagged early milestones: capture/restore/hooks/daemon/
  picker/freeze scaffolding (see the roadmap in `docs/DESIGN.md`)

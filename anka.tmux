#!/usr/bin/env bash
# tmux-anka — TPM entrypoint.
# Resolves the binary inside the plugin dir (never touches PATH), installs it if
# missing/stale, then registers keybindings, auto-save hooks, auto-restore and
# the status refresh.

set -u

CURRENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

resolve_binary() {
    if [ -x "$CURRENT_DIR/bin/anka" ]; then
        echo "$CURRENT_DIR/bin/anka"
    elif [ -x "$CURRENT_DIR/target/release/anka" ]; then
        echo "$CURRENT_DIR/target/release/anka"
    fi
}

VERSION="$(grep -m1 '^version' "$CURRENT_DIR/Cargo.toml" | sed -E 's/.*"(.*)".*/\1/')"
BINARY="$(resolve_binary)"

needs_install() {
    [ -z "$BINARY" ] && return 0
    local have
    have="$("$BINARY" --version 2>/dev/null | awk '{print $2}')"
    [ "$have" != "$VERSION" ]
}

if needs_install; then
    tmux display-message "tmux-anka: installing binary…" 2>/dev/null || true
    "$CURRENT_DIR/scripts/install-binary.sh" >/dev/null 2>&1
    BINARY="$(resolve_binary)"
fi

if [ -z "$BINARY" ]; then
    tmux display-message "tmux-anka: binary could not be installed (need cargo or a release asset)" 2>/dev/null || true
    exit 0
fi

opt() { tmux show-options -gqv "$1"; }

SAVE_KEY="$(opt @anka-save-key)";       SAVE_KEY="${SAVE_KEY:-C-s}"
RESTORE_KEY="$(opt @anka-restore-key)"; RESTORE_KEY="${RESTORE_KEY:-C-r}"
PICK_KEY="$(opt @anka-pick-key)";       PICK_KEY="${PICK_KEY:-P}"

# ── Keybindings ──────────────────────────────────────────────────────────────
tmux bind-key "$SAVE_KEY"    run-shell "$BINARY save"
tmux bind-key "$RESTORE_KEY" run-shell "$BINARY restore"
tmux bind-key "$PICK_KEY"    display-popup -E "$BINARY pick"

# ── Event-driven auto-save (native hooks; no status-interval piggyback) ───────
tmux set-hook -g session-closed   "run-shell \"$BINARY hook session-closed\""
tmux set-hook -g client-detached  "run-shell \"$BINARY hook client-detached\""

# ── Optional interval daemon ─────────────────────────────────────────────────
INTERVAL="$(opt @anka-save-interval)"; INTERVAL="${INTERVAL:-10}"
if [ "$INTERVAL" != "0" ]; then
    "$BINARY" daemon >/dev/null 2>&1 &
fi

# ── Auto-restore on server start (once per server, guarded inside the binary) ─
RESTORE_ON_START="$(opt @anka-restore-on-start)"; RESTORE_ON_START="${RESTORE_ON_START:-on}"
if [ "$RESTORE_ON_START" = "on" ]; then
    "$BINARY" autostart >/dev/null 2>&1 &
fi

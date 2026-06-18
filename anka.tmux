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
SWITCH_KEY="$(opt @anka-switch-key)";           SWITCH_KEY="${SWITCH_KEY:-s}"
NEW_KEY="$(opt @anka-new-key)";                 NEW_KEY="${NEW_KEY:-C}"
KILL_KEY="$(opt @anka-kill-key)";               KILL_KEY="${KILL_KEY:-X}"
PROMOTE_KEY="$(opt @anka-promote-key)";         PROMOTE_KEY="${PROMOTE_KEY:-@}"
SWITCH_NAME_KEY="$(opt @anka-switch-name-key)"; SWITCH_NAME_KEY="${SWITCH_NAME_KEY:-g}"
LAST_KEY="$(opt @anka-last-key)";               LAST_KEY="${LAST_KEY:-S}"

# ── Keybindings ──────────────────────────────────────────────────────────────
# Save/restore arka planda çalışır (run-shell -b). run-shell, komutun stdout'unu
# Enter/q ile kapatılması gereken bir view'da gösterir; bu yüzden anka'nın stdout'unu
# (>/dev/null) susturup sonucu kısa bir display-message ile mesaj satırında gösteriyoruz
# (anka ayrıca @anka_status widget'ını da günceller). stderr açık kalır ki gerçek
# hatalar yine görünebilsin.
tmux bind-key "$SAVE_KEY"    run-shell -b "$BINARY save >/dev/null && tmux display-message 'anka: snapshot saved ✔'"
tmux bind-key "$RESTORE_KEY" run-shell -b "$BINARY restore >/dev/null && tmux display-message 'anka: snapshot restored ✔'"
tmux bind-key "$PICK_KEY"    display-popup -E "$BINARY pick"

# ── Session management (replaces tmux-sessionx + tmux-sessionist) ─────────────
# Switcher: an interactive popup over live + snapshot + zoxide sessions.
tmux bind-key "$SWITCH_KEY"      display-popup -E "$BINARY switch"
# Sessionist-style quick actions. command-prompt feeds the name as %%.
tmux bind-key "$NEW_KEY"         command-prompt -p "New session:" "run-shell \"$BINARY session new '%%'\""
tmux bind-key "$KILL_KEY"        run-shell "$BINARY session kill"
tmux bind-key "$PROMOTE_KEY"     command-prompt -p "Promote pane to session:" "run-shell \"$BINARY session promote '%%'\""
tmux bind-key "$SWITCH_NAME_KEY" command-prompt -p "Switch to session:" "run-shell \"$BINARY session switch '%%'\""
tmux bind-key "$LAST_KEY"        run-shell "$BINARY session last"

# ── Event-driven auto-save (native hooks; no status-interval piggyback) ───────
tmux set-hook -g session-closed   "run-shell \"$BINARY hook session-closed >/dev/null\""
tmux set-hook -g client-detached  "run-shell \"$BINARY hook client-detached >/dev/null\""

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

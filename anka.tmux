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
SWITCH_KEY="$(opt @anka-switch-key)";           SWITCH_KEY="${SWITCH_KEY:-o}"
NEW_KEY="$(opt @anka-new-key)";                 NEW_KEY="${NEW_KEY:-C}"
KILL_KEY="$(opt @anka-kill-key)";               KILL_KEY="${KILL_KEY:-X}"
PROMOTE_KEY="$(opt @anka-promote-key)";         PROMOTE_KEY="${PROMOTE_KEY:-@}"
SWITCH_NAME_KEY="$(opt @anka-switch-name-key)"; SWITCH_NAME_KEY="${SWITCH_NAME_KEY:-g}"
LAST_KEY="$(opt @anka-last-key)";               LAST_KEY="${LAST_KEY:-S}"
MENU_KEY="$(opt @anka-menu-key)";               MENU_KEY="${MENU_KEY:-F}"

# ── Keybindings ──────────────────────────────────────────────────────────────
# Set any @anka-*-key to 'none' to skip that binding (keep your own).
bind_anka() { [ "$1" = none ] && return 0; tmux bind-key "$@"; }

# Save/restore arka planda çalışır (run-shell -b). run-shell, komutun stdout'unu
# Enter/q ile kapatılması gereken bir view'da gösterir; bu yüzden anka'nın stdout'unu
# (>/dev/null) susturup sonucu kısa bir display-message ile mesaj satırında gösteriyoruz
# (anka ayrıca @anka_status widget'ını da günceller). stderr açık kalır ki gerçek
# hatalar yine görünebilsin.
bind_anka "$SAVE_KEY"    run-shell -b "$BINARY save >/dev/null && tmux display-message 'anka: snapshot saved ✔'"
bind_anka "$RESTORE_KEY" run-shell -b "$BINARY restore >/dev/null && tmux display-message 'anka: snapshot restored ✔'"
bind_anka "$PICK_KEY"    display-popup -E "$BINARY pick"

# ── Session management (replaces tmux-sessionx + tmux-sessionist) ─────────────
# Switcher: an interactive popup over live + snapshot + zoxide sessions.
bind_anka "$SWITCH_KEY"      display-popup -E "$BINARY switch"
# Sessionist-style quick actions. command-prompt feeds the name as %%.
bind_anka "$NEW_KEY"         command-prompt -p "New session:" "run-shell \"$BINARY session new '%%'\""
bind_anka "$KILL_KEY"        run-shell "$BINARY session kill"
bind_anka "$PROMOTE_KEY"     command-prompt -p "Promote pane to session:" "run-shell \"$BINARY session promote '%%'\""
bind_anka "$SWITCH_NAME_KEY" command-prompt -p "Switch to session:" "run-shell \"$BINARY session switch '%%'\""
bind_anka "$LAST_KEY"        run-shell "$BINARY session last"

# Action menu (replaces tmux-fzf). run-shell captures the invoking client/session
# (#{...} expands here, not in display-popup -E); anka reopens itself in a popup.
bind_anka "$MENU_KEY"        run-shell -b "$BINARY menu --client '#{client_name}' --session '#{session_name}'"

# ── Event-driven auto-save (native hooks; no status-interval piggyback) ───────
# NOT session-closed: it fires after the session is already gone, so saving there
# prunes it from the snapshot — and cascades on logout/shutdown as every session
# tears down, losing them after a reboot. Unset it (clears any stale hook too).
tmux set-hook -gu session-closed
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

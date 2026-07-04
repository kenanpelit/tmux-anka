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
GRAB_KEY="$(opt @anka-grab-key)";               GRAB_KEY="${GRAB_KEY:-Space}"
SEARCH_KEY="$(opt @anka-search-key)";           SEARCH_KEY="${SEARCH_KEY:-m}"

# Popup size for the full-screen TUIs (switch/pick). Override with
# @anka-popup-width / @anka-popup-height (any display-popup size: 80%, 120, …).
POPUP_W="$(opt @anka-popup-width)";  POPUP_W="${POPUP_W:-80%}"
POPUP_H="$(opt @anka-popup-height)"; POPUP_H="${POPUP_H:-75%}"

# ── Keybindings ──────────────────────────────────────────────────────────────
# Set any @anka-*-key to 'none' to skip that binding (keep your own).
# $BINARY is single-quoted inside every run-shell/display-popup string so a
# plugin path containing spaces survives the shell that executes it. Inside
# tmux the binary reports results via display-message itself and keeps stdout
# quiet, so plain `run-shell -b` is enough (no view to dismiss, no >/dev/null).
bind_anka() { [ "$1" = none ] && return 0; tmux bind-key "$@"; }

bind_anka "$SAVE_KEY"    run-shell -b "'$BINARY' save"
bind_anka "$RESTORE_KEY" run-shell -b "'$BINARY' restore"
bind_anka "$PICK_KEY"    display-popup -w "$POPUP_W" -h "$POPUP_H" -E "'$BINARY' pick"

# ── Session management (replaces tmux-sessionx + tmux-sessionist) ─────────────
# Switcher: an interactive popup over live + snapshot + zoxide sessions.
bind_anka "$SWITCH_KEY"      display-popup -w "$POPUP_W" -h "$POPUP_H" -E "'$BINARY' switch"
# Sessionist-style quick actions. new/promote/switch prompt for the name
# themselves (--prompt: a raw-mode inline prompt in a small popup), so typed
# input never rides through a shell string — no command-prompt '%%' injection.
bind_anka "$NEW_KEY"         display-popup -w 40 -h 3 -E "'$BINARY' session new --prompt"
bind_anka "$KILL_KEY"        run-shell -b "'$BINARY' session kill"
bind_anka "$PROMOTE_KEY"     display-popup -w 40 -h 3 -E "'$BINARY' session promote --prompt"
bind_anka "$SWITCH_NAME_KEY" display-popup -w 40 -h 3 -E "'$BINARY' session switch --prompt"
bind_anka "$LAST_KEY"        run-shell -b "'$BINARY' session last"

# Action menu (replaces tmux-fzf). run-shell captures the invoking client
# (#{...} expands here, not in display-popup -E); anka reopens itself in a
# popup. Client names are tty paths (safe charset) but stay single-quoted.
bind_anka "$MENU_KEY"        run-shell -b "'$BINARY' menu --client '#{client_name}'"

# Screen-text tools (replace extrakto + tmux-fuzzback). run-shell captures the
# pane/client (#{...} expands here, not in display-popup -E) → anka picker popup.
bind_anka "$GRAB_KEY"        run-shell -b "'$BINARY' grab --pane '#{pane_id}' --client '#{client_name}'"
bind_anka "$SEARCH_KEY"      run-shell -b "'$BINARY' search --pane '#{pane_id}' --client '#{client_name}'"

# ── Event-driven auto-save (native hooks; no status-interval piggyback) ───────
# NOT session-closed: it fires after the session is already gone, so saving there
# prunes it from the snapshot — and cascades on logout/shutdown as every session
# tears down, losing them after a reboot. (Anka <0.9.4 set it itself; if you
# upgraded from that far back, run `tmux set-hook -gu session-closed` once.)
# Indexed hook slot [27]: coexists with user/plugin client-detached hooks
# instead of clobbering the global slot.
#
# anka <=0.12.x set this hook unindexed, which tmux stores at [0] and keeps
# alongside [27] — both would fire on every detach. Unset [0] only when it is
# recognizably ours, so a genuine user hook there survives. (Note:
# `show-hooks -g` without a hook name lists only hook NAMES; the values need
# the hook asked for explicitly.)
legacy_hook="$(tmux show-hooks -g client-detached 2>/dev/null | grep '^client-detached\[0\] ' || true)"
case "$legacy_hook" in
    *anka*client-detached*) tmux set-hook -gu 'client-detached[0]' ;;
esac
tmux set-hook -g 'client-detached[27]' "run-shell -b \"'$BINARY' hook client-detached\""

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

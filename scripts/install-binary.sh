#!/usr/bin/env bash
# Install the anka binary into the plugin directory: try a prebuilt release
# asset first (checksum-verified), fall back to compiling with cargo. Never
# writes to PATH.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

REPO="kenanpelit/tmux-anka"
VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')"

# Replace bin/anka atomically: stage next to it, then rename. The interval
# daemon may be running bin/anka — rename swaps the dirent out from under it
# (a direct cp onto the running binary would fail with ETXTBSY).
install_atomic() {
    local src="$1" staged="$ROOT/bin/.anka.tmp.$$"
    mkdir -p bin
    cp "$src" "$staged" || return 1
    chmod 755 "$staged"
    mv -f "$staged" bin/anka || { rm -f "$staged"; return 1; }
    [ -x bin/anka ]
}

try_download() {
    local arch os asset base tmpdir
    arch="$(uname -m)"
    os="$(uname -s | tr '[:upper:]' '[:lower:]')"
    asset="anka-${VERSION}-${arch}-${os}.tar.gz"
    base="https://github.com/${REPO}/releases/download/v${VERSION}"
    command -v curl >/dev/null 2>&1 || return 1
    command -v sha256sum >/dev/null 2>&1 || return 1
    tmpdir="$(mktemp -d)" || return 1
    # shellcheck disable=SC2064  # expand now: tmpdir is local to this function
    trap "rm -rf '$tmpdir' '$ROOT/bin/.anka.tmp.$$'" EXIT
    curl -fsSL "$base/$asset" -o "$tmpdir/$asset" 2>/dev/null || return 1
    curl -fsSL "$base/SHA256SUMS" -o "$tmpdir/SHA256SUMS" 2>/dev/null || return 1
    grep -F "$asset" "$tmpdir/SHA256SUMS" > "$tmpdir/SHA256SUMS.asset" || return 1
    (cd "$tmpdir" && sha256sum -c --status SHA256SUMS.asset) || return 1
    tar -xzf "$tmpdir/$asset" -C "$tmpdir" 2>/dev/null || return 1
    [ -x "$tmpdir/anka" ] || return 1
    install_atomic "$tmpdir/anka"
}

try_compile() {
    command -v cargo >/dev/null 2>&1 || return 1
    cargo build --release
    install_atomic target/release/anka
}

if try_download; then
    echo "anka: installed prebuilt binary v${VERSION}"
elif try_compile; then
    echo "anka: compiled binary v${VERSION}"
else
    echo "anka: could not install (no verified release asset and no cargo)" >&2
    exit 1
fi

#!/usr/bin/env bash
# Install the anka binary into the plugin directory: try a prebuilt release
# asset first, fall back to compiling with cargo. Never writes to PATH.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

REPO="kenanpelit/tmux-anka"
VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')"

try_download() {
    local arch os asset url
    arch="$(uname -m)"
    os="$(uname -s | tr '[:upper:]' '[:lower:]')"
    asset="anka-${VERSION}-${arch}-${os}.tar.gz"
    url="https://github.com/${REPO}/releases/download/v${VERSION}/${asset}"
    command -v curl >/dev/null 2>&1 || return 1
    mkdir -p bin
    curl -fsSL "$url" -o /tmp/anka-dl.tgz 2>/dev/null || return 1
    tar -xzf /tmp/anka-dl.tgz -C bin 2>/dev/null || return 1
    [ -x bin/anka ]
}

try_compile() {
    command -v cargo >/dev/null 2>&1 || return 1
    cargo build --release
    mkdir -p bin
    cp target/release/anka bin/anka
}

if try_download; then
    echo "anka: installed prebuilt binary v${VERSION}"
elif try_compile; then
    echo "anka: compiled binary v${VERSION}"
else
    echo "anka: could not install (no release asset and no cargo)" >&2
    exit 1
fi

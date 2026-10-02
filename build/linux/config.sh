#!/usr/bin/env bash
set -euo pipefail

BALLER_SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BALLER_VERSION="${BALLER_VERSION:-$(grep '^version' "$BALLER_SRC_DIR/Cargo.toml" | head -1 | cut -d'"' -f2)}"
BALLER_TARGET="${BALLER_TARGET:-x86_64-unknown-linux-gnu}"
BALLER_PROFILE="${BALLER_PROFILE:-debug}"
BALLER_OUTPUT_DIR="${BALLER_OUTPUT_DIR:-$(dirname "${BASH_SOURCE[0]}")/dist}"

# Where `install` puts the binary and `uninstall` looks for it.
#
# Two destinations, because they answer different questions. The system
# directory is where a package puts a tool so every user gets it, and it is
# writable only with root. The user directory is writable by the person
# running the script. Both are on PATH, and whichever comes first wins, so
# installing to only one of them is how a machine ends up with two `baller`
# binaries that disagree — the fresh one silently shadowed by the stale one.
# Writing to both keeps them identical, and makes either one a safe fallback
# when the other cannot be written.
BALLER_SYSTEM_INSTALL_DIR="${BALLER_SYSTEM_INSTALL_DIR:-/usr/local/bin}"
BALLER_USER_INSTALL_DIR="${BALLER_USER_INSTALL_DIR:-$HOME/.local/bin}"

# `BALLER_INSTALL_DIR` predates the pair and still wins outright: naming one
# location is an explicit request for that location, so honour it and do not
# fan out to the others.
if [ -n "${BALLER_INSTALL_DIR:-}" ]; then
    BALLER_INSTALL_DIRS=("$BALLER_INSTALL_DIR")
elif [ "$BALLER_SYSTEM_INSTALL_DIR" = "$BALLER_USER_INSTALL_DIR" ]; then
    BALLER_INSTALL_DIRS=("$BALLER_SYSTEM_INSTALL_DIR")
else
    BALLER_INSTALL_DIRS=("$BALLER_SYSTEM_INSTALL_DIR" "$BALLER_USER_INSTALL_DIR")
fi

CARGO_FLAGS=()

if [ "$BALLER_PROFILE" = "release" ]; then
    CARGO_FLAGS+=("--release")
fi
if [ -n "$BALLER_TARGET" ]; then
    CARGO_FLAGS+=("--target" "$BALLER_TARGET")
fi
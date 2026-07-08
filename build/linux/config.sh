#!/usr/bin/env bash
set -euo pipefail

BALLER_SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BALLER_VERSION="${BALLER_VERSION:-$(grep '^version' "$BALLER_SRC_DIR/Cargo.toml" | head -1 | cut -d'"' -f2)}"
BALLER_TARGET="${BALLER_TARGET:-x86_64-unknown-linux-gnu}"
BALLER_PROFILE="${BALLER_PROFILE:-debug}"
BALLER_OUTPUT_DIR="${BALLER_OUTPUT_DIR:-$(dirname "${BASH_SOURCE[0]}")/dist}"
BALLER_INSTALL_DIR="${BALLER_INSTALL_DIR:-/usr/local/bin}"
CARGO_FLAGS=()

if [ "$BALLER_PROFILE" = "release" ]; then
    CARGO_FLAGS+=("--release")
fi
if [ -n "$BALLER_TARGET" ]; then
    CARGO_FLAGS+=("--target" "$BALLER_TARGET")
fi
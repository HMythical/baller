#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/config.sh"

function run_install() {
    local binary_name="baller"
    local release_binary="$BALLER_SRC_DIR/target/$BALLER_TARGET/release/$binary_name"
    local install_binary="$BALLER_INSTALL_DIR/$binary_name"
    
    mkdir -p "$BALLER_INSTALL_DIR"
    
    if [ ! -f "$release_binary" ]; then
        echo "Binary not found. Running release build first..."
        "$SCRIPT_DIR/build.sh" release
    fi
    
    if [ -f "$install_binary" ]; then
        echo "Removing existing $install_binary"
        rm -f "$install_binary"
    fi
    
    echo "Installing $release_binary to $install_binary"
    cp "$release_binary" "$install_binary"
    
    if command -v ldconfig >/dev/null 2>&1; then
        echo "Running ldconfig..."
        ldconfig
    fi
    
    echo "Installed successfully"
}

run_install
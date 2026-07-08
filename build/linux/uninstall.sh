#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/config.sh"

function run_uninstall() {
    local binary_name="baller"
    local install_binary="$BALLER_INSTALL_DIR/$binary_name"
    
    if [ -f "$install_binary" ]; then
        echo "Removing $install_binary"
        rm -f "$install_binary"
        
        if command -v ldconfig >/dev/null 2>&1; then
            echo "Running ldconfig..."
            ldconfig
        fi
        
        echo "Uninstalled successfully"
    else
        echo "Binary not found at $install_binary"
    fi
}

run_uninstall
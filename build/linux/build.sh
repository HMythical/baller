#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_FILE="$SCRIPT_DIR/config.sh"

if [ ! -f "$CONFIG_FILE" ]; then
    echo "Error: config.sh not found at $CONFIG_FILE" >&2
    exit 1
fi

source "$CONFIG_FILE"

function print_help() {
    echo "Baller Build System"
    echo ""
    echo "Usage: $0 <command>"
    echo ""
    echo "Commands:"
    echo "  dev      Build in debug mode"
    echo "  release  Build in release mode (stripped binary)"
    echo "  test     Run tests and linting"
    echo "  clean    Clean build artifacts"
    echo "  dist     Create distribution packages"
    echo "  install  Install binary to system"
    echo "  uninstall Remove binary from system"
    echo "  help     Show this help"
    echo ""
}

function run_build() {
    local cargo_cmd="cargo build ${CARGO_FLAGS[@]}"
    
    echo "Running: $cargo_cmd"
    eval "$cargo_cmd"
}

function run_test() {
    echo "Running tests..."
    cargo test
    
    echo "Running clippy..."
    cargo clippy -- -D warnings
    
    echo "Checking formatting..."
    cargo fmt --check
}

function run_clean() {
    echo "Cleaning build artifacts..."
    cargo clean
    
    if [ -d "$BALLER_OUTPUT_DIR" ]; then
        echo "Removing $BALLER_OUTPUT_DIR"
        rm -rf "$BALLER_OUTPUT_DIR"
    fi
}

function run_release() {
    BALLER_PROFILE="release"
    CARGO_FLAGS=("--release")
    if [ -n "$BALLER_TARGET" ]; then
        CARGO_FLAGS+=("--target" "$BALLER_TARGET")
    fi
    run_build
    
    local binary_name="baller"
    local binary_path="$BALLER_SRC_DIR/target/${BALLER_TARGET}/${BALLER_PROFILE}/$binary_name"
    
    if [ "$BALLER_PROFILE" = "release" ]; then
        local output_binary="$BALLER_OUTPUT_DIR/$binary_name"
        mkdir -p "$BALLER_OUTPUT_DIR"
        
        if command -v strip >/dev/null 2>&1; then
            echo "Stripping binary..."
            strip "$binary_path" -o "$output_binary"
        else
            echo "Warning: strip not found, copying without stripping"
            cp "$binary_path" "$output_binary"
        fi
        
        echo "Release binary at: $output_binary"
    fi
}

function run_dist() {
    local package_type=""
    
    for arg in "${@:2}"; do
        case $arg in
            --deb)
                package_type="deb"
                ;;
            --rpm)
                package_type="rpm"
                ;;
            --appimage)
                package_type="appimage"
                ;;
        esac
    done
    
    run_release
    
    case $package_type in
        deb)
            echo "Creating .deb package..."
            echo "Warning: .deb package creation not implemented"
            ;;
        rpm)
            echo "Creating .rpm package..."
            echo "Warning: .rpm package creation not implemented"
            ;;
        appimage)
            echo "Creating AppImage..."
            echo "Warning: AppImage creation not implemented"
            ;;
        *)
            echo "Creating .tar.gz archive..."
            mkdir -p "$BALLER_OUTPUT_DIR"
            tar -czf "$BALLER_OUTPUT_DIR/baller.tar.gz" -C "$BALLER_SRC_DIR/target/$BALLER_TARGET/$BALLER_PROFILE" baller
            echo "Archive at: $BALLER_OUTPUT_DIR/baller.tar.gz"
            ;;
    esac
}

function run_install() {
    local binary_name="baller"
    local source_binary="$BALLER_SRC_DIR/target/$BALLER_TARGET/$BALLER_PROFILE/$binary_name"
    local install_binary="$BALLER_INSTALL_DIR/$binary_name"
    
    mkdir -p "$BALLER_INSTALL_DIR"
    
    if [ ! -f "$source_binary" ]; then
        echo "Error: Binary not found at $source_binary"
        echo "Run './build.sh release' first"
        exit 1
    fi
    
    if [ -f "$install_binary" ]; then
        echo "Removing existing $install_binary"
        rm -f "$install_binary"
    fi
    
    echo "Installing $source_binary to $install_binary"
    cp "$source_binary" "$install_binary"
    
    if command -v ldconfig >/dev/null 2>&1; then
        echo "Running ldconfig..."
        ldconfig
    fi
    
    echo "Installed successfully"
}

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

if [ $# -eq 0 ]; then
    print_help
    exit 0
fi

case "$1" in
    dev)
        run_build
        ;;
    release)
        run_release
        ;;
    test)
        run_test
        ;;
    clean)
        run_clean
        ;;
    dist)
        run_dist "$@"
        ;;
    install)
        run_install
        ;;
    uninstall)
        run_uninstall
        ;;
    help|*)
        print_help
        ;;
    *)
        echo "Unknown command: $1"
        print_help
        exit 1
        ;;
esac
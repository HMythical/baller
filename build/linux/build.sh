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

# Cargo finds its project (and rustup its toolchain file) by walking up from
# the working directory, not from this script. Run from anywhere else, a bare
# `cargo` either finds no project or finds some other one — and `clean` then
# wipes that project's target/ and reports success. Always run from the repo.
function run_cargo() {
    (cd "$BALLER_SRC_DIR" && cargo "$@")
}

function run_build() {
    echo "Running: cargo build ${CARGO_FLAGS[*]}"
    run_cargo build "${CARGO_FLAGS[@]}"
}

function run_test() {
    echo "Running tests..."
    run_cargo test

    echo "Running clippy..."
    run_cargo clippy --all-targets -- -D warnings

    echo "Checking formatting..."
    run_cargo fmt --check
}

function run_clean() {
    echo "Cleaning build artifacts..."

    # A build run as another user — `sudo ./build.sh …`, or a container with
    # the repo mounted — leaves directories in target/ this user cannot write,
    # and cargo stops at the first file inside one with a bare "Permission
    # denied (os error 13)", after removing an arbitrary part of the tree.
    # Look before deleting anything, and say how to get the files back.
    local target_dir="${CARGO_TARGET_DIR:-target}"
    case "$target_dir" in
        /*) ;;
        *) target_dir="$BALLER_SRC_DIR/$target_dir" ;;
    esac

    local blocked
    blocked="$(find "$target_dir" -type d ! -writable ! -empty -print -quit 2>/dev/null)" || true
    if [ -n "$blocked" ]; then
        echo "Error: cannot clean $target_dir: $blocked is not writable by $(id -un)." >&2
        echo "A build was run as another user (usually root, via sudo or a container)." >&2
        echo "Take the files back, then re-run clean:" >&2
        echo "  sudo chown -R \"$(id -un):\" \"$target_dir\"" >&2
        return 1
    fi

    run_cargo clean

    if [ -d "$BALLER_OUTPUT_DIR" ]; then
        echo "Removing $BALLER_OUTPUT_DIR"
        rm -rf "$BALLER_OUTPUT_DIR"
    fi
}

function run_release() {
    export BALLER_PROFILE="release"
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
    local release_binary="$BALLER_SRC_DIR/target/$BALLER_TARGET/release/$binary_name"

    if [ ! -f "$release_binary" ]; then
        echo "Binary not found. Running release build first..."
        run_release
    fi

    if [ ! -f "$release_binary" ]; then
        echo "Error: no release binary at $release_binary" >&2
        return 1
    fi

    # A failure per directory is a privilege problem, not a build problem: the
    # system directory needs root, the user directory never does. So install to
    # whichever ones we can and name the ones we could not, rather than letting
    # one unwritable directory abandon the rest and leave no `baller` at all.
    local installed=0
    local skipped=""

    for install_dir in "${BALLER_INSTALL_DIRS[@]}"; do
        local install_binary="$install_dir/$binary_name"

        if mkdir -p "$install_dir" 2>/dev/null && cp "$release_binary" "$install_binary" 2>/dev/null; then
            chmod +x "$install_binary" 2>/dev/null || true
            installed=$((installed + 1))
            echo "Installed $install_binary"
        else
            skipped="$skipped $install_dir"
        fi
    done

    # Only a machine-wide install could affect the dynamic linker cache, and
    # refreshing it needs root. `baller` is not a shared library, so a failure
    # here changes nothing and must not fail the install.
    if command -v ldconfig >/dev/null 2>&1; then
        ldconfig 2>/dev/null || true
    fi

    if [ -n "$skipped" ]; then
        echo ""
        echo "Skipped (needs elevated privileges):$skipped" >&2
        if [ "$installed" -eq 0 ]; then
            echo "Error: nothing was installed" >&2
            return 1
        fi
        echo "Re-run with sudo to install to the system directory as well." >&2
    fi
}

function run_uninstall() {
    local binary_name="baller"
    local removed=0

    for install_dir in "${BALLER_INSTALL_DIRS[@]}"; do
        local install_binary="$install_dir/$binary_name"

        if [ -f "$install_binary" ]; then
            if rm -f "$install_binary" 2>/dev/null; then
                removed=$((removed + 1))
                echo "Removed $install_binary"
            else
                echo "Skipped $install_binary (needs elevated privileges)" >&2
            fi
        fi
    done

    if command -v ldconfig >/dev/null 2>&1; then
        ldconfig 2>/dev/null || true
    fi

    if [ "$removed" -eq 0 ]; then
        echo "No baller binary found in: ${BALLER_INSTALL_DIRS[*]}"
        return 1
    fi

    echo "Uninstalled successfully"
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
    help)
        print_help
        ;;
    *)
        echo "Unknown command: $1" >&2
        print_help >&2
        exit 1
        ;;
esac

#!/usr/bin/env bash
# Convenience wrapper. The uninstall logic lives in `build.sh` so that this
# script and `./build.sh uninstall` can never disagree about where the binary
# was put; delegating is the whole point of keeping this file.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

exec "$SCRIPT_DIR/build.sh" uninstall

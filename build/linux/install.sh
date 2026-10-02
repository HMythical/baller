#!/usr/bin/env bash
# Convenience wrapper. The install logic lives in `build.sh` so that this
# script and `./build.sh install` can never disagree about where the binary
# goes; delegating is the whole point of keeping this file.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

exec "$SCRIPT_DIR/build.sh" install

#!/bin/bash
# Check Kruxia Flow binary size against 15MB target

set -e

# Resolve the cargo target dir (honors CARGO_TARGET_DIR / build.target-dir)
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version=1 --no-deps 2>/dev/null | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}"

BINARY_PATH="${1:-${CARGO_TARGET_DIR}/release/kruxiaflow}"
TARGET_SIZE_MB=15

if [ ! -f "$BINARY_PATH" ]; then
    echo "ERROR: Binary not found at $BINARY_PATH"
    echo "Run: cargo build --release"
    exit 1
fi

# Get file size (works on both macOS and Linux)
if [[ "$OSTYPE" == "darwin"* ]]; then
    SIZE_BYTES=$(stat -f%z "$BINARY_PATH")
else
    SIZE_BYTES=$(stat -c%s "$BINARY_PATH")
fi

SIZE_MB=$((SIZE_BYTES / 1024 / 1024))
SIZE_KB=$((SIZE_BYTES / 1024))

echo "Binary: $BINARY_PATH"
echo "Size: ${SIZE_MB}MB (${SIZE_KB}KB)"
echo "Target: <${TARGET_SIZE_MB}MB"

if [ $SIZE_MB -ge $TARGET_SIZE_MB ]; then
    echo "❌ FAIL: Binary size (${SIZE_MB}MB) exceeds ${TARGET_SIZE_MB}MB target"
    exit 1
else
    echo "✅ PASS: Binary size is within target"
    exit 0
fi

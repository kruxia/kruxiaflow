#!/bin/bash

set -e

export PROJECT_DIR=$(dirname $(dirname "$(realpath "$0")"))
export PROFILE_DIR="${PROJECT_DIR}/var/memory"
export BINARY="${PROJECT_DIR}/target/profiling/kruxiaflow"

FINAL_DUMP=$(ls -t ${PROFILE_DIR}/jeprof.out.*.heap | head -1)
echo "Final heap dump: $FINAL_DUMP"

# On macOS, use dSYM if available; on Linux, use binary directly
if [ -d "${BINARY}.dSYM" ]; then
    echo "Using dSYM for symbol resolution (macOS)"
    BINARY_PATH="${BINARY}.dSYM"
else
    echo "Using binary for symbol resolution (Linux)"
    BINARY_PATH="${BINARY}"
fi

echo "Generating allocation report..."
jeprof --show_bytes --text "$BINARY_PATH" "$FINAL_DUMP" > ${PROFILE_DIR}/allocation_report.txt

echo "Generating call graph SVG..."
jeprof --show_bytes --svg "$BINARY_PATH" "$FINAL_DUMP" > ${PROFILE_DIR}/callgraph.svg

echo "Generating call graph PDF..."
jeprof --show_bytes --pdf "$BINARY_PATH" "$FINAL_DUMP" > ${PROFILE_DIR}/callgraph.pdf

echo "Profiling complete!"
echo "Results saved to: $PROFILE_DIR"
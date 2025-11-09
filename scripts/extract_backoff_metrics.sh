#!/bin/bash
#
# Extract orchestrator backoff metrics from server logs
#
# Usage:
#   ./scripts/extract_backoff_metrics.sh <server_log_file> <output_csv>
#

if [ $# -lt 2 ]; then
    echo "Usage: $0 <server_log_file> <output_csv>"
    exit 1
fi

SERVER_LOG="$1"
OUTPUT_CSV="$2"

echo "Extracting backoff metrics from $SERVER_LOG..."

# Create CSV header
echo "timestamp,backoff_ms,event_count" > "$OUTPUT_CSV"

# Extract backoff intervals from "No events found" debug lines
grep -E "No events found, backoff interval:" "$SERVER_LOG" | \
    sed -E 's/.*([0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]+Z).*backoff interval: ([0-9.]+)(ms|s).*/\1,\2,0/' | \
    sed 's/s$/000/' | \
    sed 's/ms$//' >> "$OUTPUT_CSV"

# Extract event poll counts from "Polled N events" debug lines
grep -E "Polled [0-9]+ events, resetting backoff" "$SERVER_LOG" | \
    sed -E 's/.*([0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]+Z).*Polled ([0-9]+) events.*/\1,0,\2/' >> "$OUTPUT_CSV"

# Sort by timestamp
sort -t',' -k1 "$OUTPUT_CSV" -o "$OUTPUT_CSV"

echo "Backoff metrics saved to $OUTPUT_CSV"

# Generate summary
TOTAL_LINES=$(wc -l < "$OUTPUT_CSV")
EMPTY_POLLS=$(grep ",0$" "$OUTPUT_CSV" | wc -l)
EVENT_POLLS=$(grep -v ",0$" "$OUTPUT_CSV" | wc -l)

echo ""
echo "Summary:"
echo "  Total polls: $((TOTAL_LINES - 1))"
echo "  Empty polls: $EMPTY_POLLS"
echo "  Event polls: $EVENT_POLLS"

if [ $EVENT_POLLS -gt 0 ]; then
    RATIO=$(echo "scale=2; $EMPTY_POLLS / $EVENT_POLLS" | bc)
    echo "  Empty/Event ratio: ${RATIO}:1"
fi

#!/bin/bash
# Test the proxy's session-aware read path with debug logging.
set -e

INPUT=$(mktemp)
LOG=$(mktemp)

cat > "$INPUT" << 'JSONL'
{"jsonrpc":"2.0","method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}},"id":1}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","method":"tools/call","params":{"name":"iris_read","arguments":{"section_id":"/Users/alrik/Code/iris-rs/README.md#iris"}},"id":2}
{"jsonrpc":"2.0","method":"tools/call","params":{"name":"iris_budget","arguments":{}},"id":3}
JSONL

echo "==> Sending initialize + read + budget through proxy..."
RUST_LOG=info timeout 10 iris serve --transport stdio < "$INPUT" > /tmp/iris-test-stdout.json 2> "$LOG" || true

echo ""
echo "==> STDERR (proxy logs):"
cat "$LOG"
echo ""
echo "==> STDOUT (MCP responses):"
cat /tmp/iris-test-stdout.json

rm -f "$INPUT" "$LOG"

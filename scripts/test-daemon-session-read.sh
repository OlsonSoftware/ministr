#!/bin/bash
# Test the daemon's session-aware read endpoint directly via curl.
set -e

SOCKET=~/.iris/irisd.sock
CORPUS="multi-804d4932"
SECTION="%2FUsers%2Falrik%2FCode%2Firis-rs%2FREADME.md%23iris"

echo "==> Creating session..."
SESSION=$(curl -s --unix-socket "$SOCKET" \
  "http://localhost/api/v1/corpora/$CORPUS/sessions" \
  -X POST -H "Content-Type: application/json" -d '{}')
echo "Response: $SESSION"

SID=$(echo "$SESSION" | python3 -c "import sys,json; print(json.load(sys.stdin)['session_id'])")
echo "Session ID: $SID"

echo ""
echo "==> Budget before read:"
curl -s --unix-socket "$SOCKET" \
  "http://localhost/api/v1/corpora/$CORPUS/sessions/$SID/budget"
echo ""

echo ""
echo "==> Session-aware read:"
RESP=$(curl -s -w "\nHTTP_STATUS:%{http_code}" --unix-socket "$SOCKET" \
  "http://localhost/api/v1/corpora/$CORPUS/sessions/$SID/read/$SECTION")
echo "$RESP" | tail -1
echo "Body (first 100 chars): $(echo "$RESP" | head -1 | cut -c1-100)"

echo ""
echo "==> Budget after read:"
curl -s --unix-socket "$SOCKET" \
  "http://localhost/api/v1/corpora/$CORPUS/sessions/$SID/budget"
echo ""

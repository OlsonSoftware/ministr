#!/usr/bin/env python3
"""F5.3-d-i — fake Splunk HEC receiver for the e2e harness.

Listens on a configurable port; accepts POST requests at any path
with `Content-Type: application/json` (the cloud's SplunkHecSink
posts to `…/services/collector/event`). Each accepted request is
appended to a JSONL record file with:

    {
        "path": "/services/collector/event",
        "auth": "Splunk <token>",
        "body": "{\"event\": {…}, \"sourcetype\": \"ministr_audit\", …}"
    }

Stdlib http.server only — same architecture as the F-Test-2 webhook
receiver. Harness scripts/e2e-cloud-local.sh spawns this before the
cloud serve starts, exports MINISTR_SIEM_HEC_URL and
MINISTR_SIEM_HEC_TOKEN so the cloud's SplunkHecSink fans out here,
then asserts the file has at least one ministr_audit event by the
time the OIDC/SAML/F3.7 assertion suite has run.

Usage:
    python3 scripts/e2e-siem-hec-receiver.py <port> <output_file>
"""

from __future__ import annotations

import json
import os
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: e2e-siem-hec-receiver.py <port> <output_file>", file=sys.stderr)
        return 2
    port = int(sys.argv[1])
    out_path = sys.argv[2]
    # Truncate-on-startup so a previous run's records don't carry over.
    try:
        os.unlink(out_path)
    except FileNotFoundError:
        pass

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt: str, *args: object) -> None:
            # Silence the per-request stderr; the harness greps the
            # record file, not the live logs.
            return

        def do_GET(self) -> None:  # noqa: N802
            # Health-check path so the harness can poll for "receiver
            # is listening".
            if self.path == "/":
                self.send_response(200)
                self.send_header("Content-Type", "text/plain")
                self.send_header("Content-Length", "2")
                self.end_headers()
                self.wfile.write(b"OK")
                return
            self.send_response(404)
            self.end_headers()

        def do_POST(self) -> None:  # noqa: N802
            length = int(self.headers.get("Content-Length", "0") or "0")
            body = self.rfile.read(length) if length else b""
            record = {
                "path": self.path,
                "auth": self.headers.get("Authorization", ""),
                "body": body.decode("utf-8", errors="replace"),
            }
            with open(out_path, "a", encoding="utf-8") as f:
                f.write(json.dumps(record))
                f.write("\n")
            # Splunk HEC returns 200 with a tiny JSON ack on success.
            ack = b'{"text":"Success","code":0}'
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(ack)))
            self.end_headers()
            self.wfile.write(ack)

    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

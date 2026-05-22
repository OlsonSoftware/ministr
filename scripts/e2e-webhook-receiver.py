#!/usr/bin/env python3
"""F-Test-2 — tiny HTTP webhook receiver for the e2e harness.

Listens on a single port, records every incoming POST to a JSONL file
(one event per line, capturing the path, headers, and decoded body),
and replies 200 OK. The harness spawns this as a subprocess, fires an
audit-emitting action against the cloud, then polls the JSONL file for
the expected delivery. Cleanup kills the subprocess via the harness's
EXIT/INT/TERM trap.

Usage:
    python3 scripts/e2e-webhook-receiver.py <port> <record_file>

The record_file is overwritten at startup so each harness run sees a
clean slate.
"""

from __future__ import annotations

import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer


def main() -> int:
    if len(sys.argv) != 3:
        print(
            "usage: e2e-webhook-receiver.py <port> <record_file>",
            file=sys.stderr,
        )
        return 2

    port = int(sys.argv[1])
    record_path = sys.argv[2]
    # Truncate to start clean; the harness reads accumulated lines.
    open(record_path, "w", encoding="utf-8").close()

    class Handler(BaseHTTPRequestHandler):
        # Quiet the default stderr access-log; the harness greps the
        # JSONL file directly.
        def log_message(self, fmt: str, *args: object) -> None:
            return

        def do_POST(self) -> None:  # noqa: N802 (BaseHTTPRequestHandler API)
            length = int(self.headers.get("Content-Length", "0") or "0")
            body = self.rfile.read(length).decode("utf-8", errors="replace") if length else ""
            entry = {
                "path": self.path,
                "headers": {k.lower(): v for k, v in self.headers.items()},
                "body": body,
            }
            with open(record_path, "a", encoding="utf-8") as fh:
                fh.write(json.dumps(entry) + "\n")
                fh.flush()
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", "2")
            self.end_headers()
            self.wfile.write(b"OK")

        # Reply 200 to anything else too, so the cloud's dispatcher
        # doesn't retry-loop on an unexpected method.
        def do_GET(self) -> None:  # noqa: N802
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", "5")
            self.end_headers()
            self.wfile.write(b"alive")

    server = HTTPServer(("127.0.0.1", port), Handler)
    print(f"e2e-webhook-receiver listening on 127.0.0.1:{port} → {record_path}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

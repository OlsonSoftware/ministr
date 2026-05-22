#!/usr/bin/env python3
"""F5.3-d-iii-c — fake TCP syslog receiver for the e2e harness.

Listens on a configurable TCP port; accepts a connection, reads
newline-terminated lines, appends each as a JSONL record:

    {
        "line": "CEF:0|ministr|ministr-cloud-audit|1|oidc.login|…",
        "peer": "127.0.0.1:54321"
    }

Mirrors the harness's other fake-receiver scripts (webhook,
SIEM HEC) in shape so the test orchestration is uniform: spawn
before serve, harness asserts via JSONL inspection.

Usage:
    python3 scripts/e2e-siem-syslog-receiver.py <port> <output_file>
"""

from __future__ import annotations

import json
import socketserver
import sys


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: e2e-siem-syslog-receiver.py <port> <output_file>", file=sys.stderr)
        return 2
    port = int(sys.argv[1])
    out_path = sys.argv[2]
    # Truncate-on-startup so a previous run's records don't carry
    # over. Empty file is the well-defined zero baseline a harness
    # `wc -l` snapshot can read against.
    with open(out_path, "w", encoding="utf-8"):
        pass

    class Handler(socketserver.StreamRequestHandler):
        def handle(self) -> None:
            peer = f"{self.client_address[0]}:{self.client_address[1]}"
            # Read newline-terminated lines until the client closes.
            # ministr's dispatcher writes one CEF line per audit
            # event and then drops the socket — but we tolerate a
            # connection that streams multiple lines (a future batch
            # mode might use one connection for many lines).
            while True:
                raw = self.rfile.readline()
                if not raw:
                    break
                line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
                if not line:
                    continue
                record = {"line": line, "peer": peer}
                with open(out_path, "a", encoding="utf-8") as f:
                    f.write(json.dumps(record))
                    f.write("\n")

    class Server(socketserver.ThreadingTCPServer):
        allow_reuse_address = True
        daemon_threads = True

    Server(("127.0.0.1", port), Handler).serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

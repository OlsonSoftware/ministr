#!/usr/bin/env python3
"""F5.3-d-iii-c-udp — fake UDP syslog receiver for the e2e harness.

Listens on a configurable UDP port via `socketserver.UDPServer`.
Each datagram is the full syslog message (no length prefix); the
cloud's `dispatch_syslog_cef_udp` sends one CEF v0 line per
datagram with no trailing newline. Each accepted datagram is
appended to a JSONL record:

    {
        "line": "CEF:0|ministr|ministr-cloud-audit|1|invite.created|…",
        "peer": "127.0.0.1:54321"
    }

Mirrors `e2e-siem-syslog-receiver.py` (TCP variant) — same record
shape so harness assertions can use the same jq extractions.

Usage:
    python3 scripts/e2e-siem-syslog-udp-receiver.py <port> <output_file>
"""

from __future__ import annotations

import json
import socketserver
import sys


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: e2e-siem-syslog-udp-receiver.py <port> <output_file>", file=sys.stderr)
        return 2
    port = int(sys.argv[1])
    out_path = sys.argv[2]
    # Truncate-on-startup so a previous run's records don't carry
    # over. Empty file = well-defined zero baseline a harness
    # `wc -l` snapshot can read against.
    with open(out_path, "w", encoding="utf-8"):
        pass

    class Handler(socketserver.BaseRequestHandler):
        def handle(self) -> None:
            # request is (data: bytes, socket). One call per datagram.
            data, _sock = self.request
            line = data.decode("utf-8", errors="replace").rstrip("\r\n")
            if not line:
                return
            peer = f"{self.client_address[0]}:{self.client_address[1]}"
            record = {"line": line, "peer": peer}
            with open(out_path, "a", encoding="utf-8") as f:
                f.write(json.dumps(record))
                f.write("\n")

    class Server(socketserver.ThreadingUDPServer):
        allow_reuse_address = True
        daemon_threads = True

    Server(("127.0.0.1", port), Handler).serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

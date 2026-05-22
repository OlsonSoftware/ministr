#!/usr/bin/env python3
"""F5.3-d-iii-b-dispatch — fake S3 PUT receiver for the e2e harness.

Listens on a configurable HTTP port; accepts PUT requests at any
path. aws-sdk-s3 in path-style mode targets
`<endpoint>/<bucket>/<key>` — we parse path-style here so the
harness can verify both bucket + key from one URL.

Each accepted PUT is appended to a JSONL record:

    {
        "method": "PUT",
        "path": "/my-bucket/audit/year=2026/month=05/day=22/…-….json",
        "auth": "AWS4-HMAC-SHA256 Credential=…",
        "content_type": "application/json",
        "body": "{\"action\":\"invite.created\", …}"
    }

The receiver does NOT validate SigV4 signatures — it just records
the request. aws-sdk-s3 still signs correctly (the SDK doesn't know
the endpoint is a fake), so a real S3 deployment using the same
config would also work; the harness's assertion is that the PUT
arrived at the expected path with the expected body shape.

Returns `200 OK` with an empty body on every PUT (real S3 returns
`200 OK` with an ETag header; the SDK doesn't require ETag for
PutObject success — `PutObjectOutput` carries it as `Option<String>`
which `None` satisfies).

Usage:
    python3 scripts/e2e-siem-s3-receiver.py <port> <output_file>
"""

from __future__ import annotations

import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: e2e-siem-s3-receiver.py <port> <output_file>", file=sys.stderr)
        return 2
    port = int(sys.argv[1])
    out_path = sys.argv[2]
    with open(out_path, "w", encoding="utf-8"):
        pass

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt: str, *args: object) -> None:
            return

        def do_GET(self) -> None:  # noqa: N802
            # Health-check path so the harness can poll for ready.
            if self.path == "/":
                self.send_response(200)
                self.send_header("Content-Type", "text/plain")
                self.send_header("Content-Length", "2")
                self.end_headers()
                self.wfile.write(b"OK")
                return
            self.send_response(404)
            self.end_headers()

        def do_PUT(self) -> None:  # noqa: N802
            length = int(self.headers.get("Content-Length", "0") or "0")
            body = self.rfile.read(length) if length else b""
            record = {
                "method": "PUT",
                "path": self.path,
                "auth": self.headers.get("Authorization", ""),
                "content_type": self.headers.get("Content-Type", ""),
                "body": body.decode("utf-8", errors="replace"),
            }
            with open(out_path, "a", encoding="utf-8") as f:
                f.write(json.dumps(record))
                f.write("\n")
            self.send_response(200)
            self.send_header("Content-Length", "0")
            # Real S3 returns an `x-amz-version-id` + `ETag` header;
            # the SDK tolerates their absence on PutObject.
            self.end_headers()

    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

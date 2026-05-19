#!/bin/sh
# ministr container entrypoint.
#
# Dual mode selected by ENTRYPOINT_MODE:
#   serve  (default)  →  ministr serve --transport http   (query app)
#   index             →  ministr index                    (ACA Job, sync)
#
# The `index` mode reads corpus paths from /data/.ministr.toml (mounted
# from the Azure Files share). The query app writes/updates that config
# as the user adds corpora via the admin endpoints.
#
# Anything else is a misconfiguration; fail fast so ACA logs a clear error.
set -e

case "${ENTRYPOINT_MODE:-serve}" in
  serve)
    exec ministr serve \
      --transport http \
      --host 0.0.0.0 \
      --port "${MINISTR_PORT:-8080}" \
      "$@"
    ;;
  index)
    exec ministr index "$@"
    ;;
  *)
    echo "ministr: unknown ENTRYPOINT_MODE='${ENTRYPOINT_MODE}'" >&2
    echo "ministr: expected 'serve' or 'index'" >&2
    exit 64
    ;;
esac

#!/bin/sh
# ministr container entrypoint.
#
# Mode selected by ENTRYPOINT_MODE:
#   serve  (default)  →  ministr serve --transport http   (query + worker)
#   index             →  ministr index                    (one-shot local)
#
# PHASE6 chunk 3 retired the `indexer-worker` mode: the serve pod now
# drains `indexer_jobs` in-process via its WorkerLoop (no separate ACA
# Job replica needed). See `deploy/azure/PHASE6.md`.
#
# Anything else is a misconfiguration; fail fast so ACA logs a clear error.
set -e

case "${ENTRYPOINT_MODE:-serve}" in
  serve)
    # When MINISTR_OAUTH_ISSUER is set, enable OAuth 2.1 so every protected
    # route (mcp, admin, daemon REST, observability) requires a Bearer
    # token. Without this, an exposed cloud server is open to any caller —
    # cloud deployments MUST set the issuer. Local dev runs without it.
    oauth_args=""
    if [ -n "${MINISTR_OAUTH_ISSUER:-}" ]; then
      oauth_args="--oauth --oauth-issuer ${MINISTR_OAUTH_ISSUER}"
    fi
    # shellcheck disable=SC2086  # word-splitting on oauth_args is intentional
    exec ministr serve \
      --transport http \
      --host 0.0.0.0 \
      --port "${MINISTR_PORT:-8080}" \
      $oauth_args \
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

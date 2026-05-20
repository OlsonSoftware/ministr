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
  indexer-worker)
    # PHASE3 chunk 3 — single-shot queue-driven worker. Pops one
    # pending job from the cloud Postgres queue, runs ingestion,
    # uploads the bundle, and exits. ACA Job re-runs on cron.
    exec ministr indexer-worker "$@"
    ;;
  *)
    echo "ministr: unknown ENTRYPOINT_MODE='${ENTRYPOINT_MODE}'" >&2
    echo "ministr: expected 'serve', 'index', or 'indexer-worker'" >&2
    exit 64
    ;;
esac

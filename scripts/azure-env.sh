#!/usr/bin/env bash
# Resolve env vars Pulumi needs to open its azblob:// state backend.
#
# The Pulumi state backend is `azblob://pulumi-state`. Pulumi needs:
#   - AZURE_STORAGE_ACCOUNT     — account hosting the state container
#   - AZURE_STORAGE_KEY         — shared key (Azure AD RBAC often
#                                 lacks Blob Data perms even when the
#                                 user has Owner, so we use the key)
#   - PULUMI_CONFIG_PASSPHRASE  — encrypts secrets in self-managed
#                                 state; per-user, only you know it
#
# Usage (from a just recipe or interactive shell):
#   eval "$(./scripts/azure-env.sh)"
#
# Resolution order:
#   1. If `.env.azure` exists at the repo root, source it. Use this
#      to permanently set PULUMI_CONFIG_PASSPHRASE etc.
#   2. If AZURE_STORAGE_ACCOUNT + AZURE_STORAGE_KEY are now set, emit
#      exports and exit.
#   3. Otherwise read `.azure-env-cache` if present.
#   4. Otherwise autodetect via `az` and write the cache.
set -euo pipefail

CACHE=".azure-env-cache"
ENVFILE=".env.azure"

# Step 1 — source the user's env file if present. The exports we emit
# below intentionally re-export anything it set so they propagate to
# the calling shell via eval "$(...)".
if [ -f "${ENVFILE}" ]; then
    # shellcheck disable=SC1090
    set -a; . "./${ENVFILE}"; set +a
fi

# Pulumi self-managed backends require the passphrase env var to be
# *set* (an empty string is fine). For this stack the passphrase is
# the empty string, so default to that unless the user overrode it
# via .env.azure.
: "${PULUMI_CONFIG_PASSPHRASE=}"
export PULUMI_CONFIG_PASSPHRASE

emit() {
    [ -n "${AZURE_STORAGE_ACCOUNT:-}" ] && printf 'export AZURE_STORAGE_ACCOUNT=%q\n' "${AZURE_STORAGE_ACCOUNT}"
    [ -n "${AZURE_STORAGE_KEY:-}" ]     && printf 'export AZURE_STORAGE_KEY=%q\n' "${AZURE_STORAGE_KEY}"
    # Always emit the passphrase, even when empty — Pulumi cares that
    # it's *set*, not that it's non-empty.
    printf 'export PULUMI_CONFIG_PASSPHRASE=%q\n' "${PULUMI_CONFIG_PASSPHRASE}"
    [ -n "${PULUMI_CONFIG_PASSPHRASE_FILE:-}" ] && printf 'export PULUMI_CONFIG_PASSPHRASE_FILE=%q\n' "${PULUMI_CONFIG_PASSPHRASE_FILE}"
}

if [ -n "${AZURE_STORAGE_ACCOUNT:-}" ] && [ -n "${AZURE_STORAGE_KEY:-}" ]; then
    emit
    exit 0
fi

if [ -f "${CACHE}" ]; then
    cat "${CACHE}"
    # Re-emit the passphrase — the cache carries only the autodetected
    # storage fields, not user-managed env like the passphrase.
    printf 'export PULUMI_CONFIG_PASSPHRASE=%q\n' "${PULUMI_CONFIG_PASSPHRASE}"
    [ -n "${PULUMI_CONFIG_PASSPHRASE_FILE:-}" ] && printf 'export PULUMI_CONFIG_PASSPHRASE_FILE=%q\n' "${PULUMI_CONFIG_PASSPHRASE_FILE}"
    exit 0
fi

if ! command -v az >/dev/null 2>&1; then
    echo "echo '✗ az CLI not found and AZURE_STORAGE_ACCOUNT not set' >&2; exit 1"
    exit 0
fi

CONTAINER="pulumi-state"

# Scan every account in the current subscription for the container.
FOUND_ACCT=""
FOUND_RG=""
while IFS=$'\t' read -r ACCOUNT RG; do
    EXISTS=$(az storage container exists \
        --account-name "${ACCOUNT}" \
        --name "${CONTAINER}" \
        --auth-mode login \
        --query exists -o tsv 2>/dev/null || echo "false")
    if [ "${EXISTS}" = "true" ]; then
        FOUND_ACCT="${ACCOUNT}"
        FOUND_RG="${RG}"
        break
    fi
done < <(az storage account list --query "[].[name,resourceGroup]" -o tsv 2>/dev/null)

if [ -z "${FOUND_ACCT}" ]; then
    cat >&2 <<EOF
✗ Could not auto-detect the storage account holding the '${CONTAINER}' container.
  Set AZURE_STORAGE_ACCOUNT and AZURE_STORAGE_KEY manually:
      export AZURE_STORAGE_ACCOUNT=<your-state-account-name>
      export AZURE_STORAGE_KEY=\$(az storage account keys list --account-name <name> --query '[0].value' -o tsv)
EOF
    exit 1
fi

# Fetch the primary key.
KEY=$(az storage account keys list \
    --account-name "${FOUND_ACCT}" \
    --resource-group "${FOUND_RG}" \
    --query "[0].value" -o tsv 2>/dev/null || true)

if [ -z "${KEY}" ]; then
    cat >&2 <<EOF
✗ Resolved AZURE_STORAGE_ACCOUNT=${FOUND_ACCT} but could not read its keys.
  You may lack 'Storage Account Key Operator' / 'Owner' on that account.
  Set AZURE_STORAGE_KEY manually:
      export AZURE_STORAGE_KEY=<primary-key>
EOF
    exit 1
fi

{
    printf 'export AZURE_STORAGE_ACCOUNT=%q\n' "${FOUND_ACCT}"
    printf 'export AZURE_STORAGE_KEY=%q\n' "${KEY}"
} | tee "${CACHE}"
echo "✓ cached resolution to ${CACHE} (delete to re-detect)" >&2

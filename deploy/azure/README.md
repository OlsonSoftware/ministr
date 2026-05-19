# ministr.ai cloud — Pulumi (Azure)

Azure-native Pulumi program that stands up the **remote MCP endpoint at
`mcp.ministr.ai`**. Single-tenant v1: always-warm ACA query app +
on-demand ACA indexer job, sharing one Azure Files mount.

## Architecture (recap)

```
GitHub webhook ─► /webhook/github (HMAC) ─►┐
                                            │
   Tauri / MCP clients ─ HTTPS/SSE ─► [Query App] ─ reads ─► [Azure Files /data]
                                            │                    ▲ writes
                                            │                    │
                       trigger indexer ────►│       [Indexer Job] ┘
```

| Resource | Pulumi type | Sizing | Monthly |
|---|---|---|---|
| Resource Group | `azure-native.resources.ResourceGroup` | — | $0 |
| Log Analytics | `azure-native.operationalinsights.Workspace` | 30-day retention, 5 GiB/mo free | ~$0 |
| Container Apps Env | `azure-native.app.ManagedEnvironment` | Consumption | $0 |
| ACR | `azure-native.containerregistry.Registry` | Basic SKU, admin user | $5 |
| Storage Account | `azure-native.storage.StorageAccount` | Standard LRS, hot | <$1 |
| File Share | `azure-native.storage.FileShare` | 10 GiB quota | ~$0.60 |
| Env Storage mount | `azure-native.app.ManagedEnvironmentsStorage` | SMB ReadWrite | $0 |
| Query Container App | `azure-native.app.ContainerApp` | 0.5 vCPU / 1 GiB, min=1/max=1 | ~$14 |
| Indexer Job | `azure-native.app.Job` | 4 vCPU / 8 GiB, manual trigger | ~$3 (typical) |
| App Insights | `azure-native.insights.Component` | linked to Log Analytics | ~$2 |
| Managed Cert | `azure-native.app.ManagedCertificate` | for `mcp.ministr.ai` | $0 |
| **Total** |  |  | **~$24–26** |

## First-time setup

```sh
cd deploy/azure
npm ci
pulumi login                       # or `pulumi login --local` for state-only
az login
az account set --subscription "<your sub id>"
pulumi stack init prod
```

### Configure secrets

```sh
# Generate a random hex string for the GitHub webhook secret:
pulumi config set --secret githubWebhookSecret "$(openssl rand -hex 32)"
```

If you want a custom domain (recommended), confirm DNS first:

```sh
pulumi config set customDomain mcp.ministr.ai
```

Then, **before `pulumi up`**, add a CNAME in your DNS pointing
`mcp.ministr.ai` → `<env>.<region>.azurecontainerapps.io` (the value is
printed as `appFqdn` after the first apply without the custom domain,
or you can guess it: `ministr-app.<random>.eastus.azurecontainerapps.io`).
ACA validates ownership via that CNAME before issuing the managed cert.

### Preview + apply

```sh
npm run preview                    # dry-run
pulumi up                          # provision (~5 min)
```

### Push the container image

The first apply succeeds but the Container App can't pull yet — the ACR
is empty. Push the image, then ACA auto-rolls:

```sh
# Build (from repo root)
docker build -t ministr:latest .

# Push to the ACR Pulumi just created
REGISTRY=$(pulumi -C deploy/azure stack output registryServer)
az acr login --name "${REGISTRY%%.*}"
docker tag ministr:latest "$REGISTRY/ministr:latest"
docker push "$REGISTRY/ministr:latest"
```

### Smoke test

```sh
APP_URL=$(pulumi -C deploy/azure stack output appFqdn)
curl "https://$APP_URL/healthz"
# → {"status":"ready","corpus_count":0,"version":"…"}
```

Once `mcp.ministr.ai` resolves and the managed cert is provisioned
(~5 min after `pulumi up`):

```sh
curl https://mcp.ministr.ai/healthz
```

## Env vars the container expects

| Var | Source | Why |
|---|---|---|
| `MINISTR_CLOUD_DATA_DIR` | hardcoded `/data` | Tells `cmd_serve_http` to use SQLite-backed persistence. |
| `MINISTR_GITHUB_WEBHOOK_SECRET` | `secrets[github-webhook-secret]` | HMAC key for the GitHub webhook. |
| `APPLICATIONINSIGHTS_CONNECTION_STRING` | `secrets[appinsights-connection-string]` | Sends traces/metrics to App Insights. |
| `RUST_LOG` | `info,ministr=debug` | Log filter. |

## Tear down

```sh
pulumi destroy
```

Deletes everything in the resource group. Custom-domain DNS record is
**yours** — Pulumi never touches it.

## What this stack does *not* do (deferred)

- **Cosmos DB.** SQLite over Azure Files works fine for single-replica
  v1. Cosmos becomes useful once we go multi-replica.
- **Managed identity for ACR.** Admin credentials are simpler for v1;
  swap to system-assigned identity + `AcrPull` role assignment in v2.
- **Custom VNet.** Public env is fine for the public MCP endpoint.
- **Backups of the share.** Azure Files snapshots are free but the
  schedule isn't in IaC yet.
- **Cost-cap alerts.** Set a budget alert via the Azure portal once the
  stack is up.

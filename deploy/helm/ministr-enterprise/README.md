# ministr-enterprise Helm chart

F5.4-c — license-gated on-prem deployment of ministr-cloud for
Kubernetes. Customer-installable; ministr never touches your cluster.

## Prerequisites

- Kubernetes 1.27 or newer
- Helm 3.x
- Your own Postgres 15+ (Cloud SQL / RDS / Azure Database for PG / on-prem)
- A ministr Enterprise license (key JWT + RS256 public key) — request from
  `support@ministr.ai`

## Install

Minimum required values:

```yaml
# values.local.yaml
license:
  key: |
    eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJlbnRlcnByaXNlX2lkIjoiYWNtZSIsLi4u...
  publicKey: |
    -----BEGIN PUBLIC KEY-----
    MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA...
    -----END PUBLIC KEY-----
postgres:
  url: "postgres://ministr:s3cret@postgres.example.com:5432/ministr?sslmode=require"
```

```bash
helm install ministr-enterprise ./deploy/helm/ministr-enterprise \
  --namespace ministr --create-namespace \
  -f values.local.yaml
```

The chart's `required:` template enforcement fails fast at
`helm install` time if any of `license.key`, `license.publicKey`, or
`postgres.url` is empty — you won't get a half-deployed pod.

## What the chart deploys

- One `Deployment` running the `ghcr.io/ministr-ai/cloud` image
  (override via `image.repository` + `image.tag`).
- One `Service` of type `ClusterIP` exposing the serve on port 8080.
- One `Secret` carrying `MINISTR_LICENSE_KEY`, `MINISTR_LICENSE_PUBLIC_KEY`,
  and `MINISTR_PG_URL`.
- Liveness + readiness probes on `/healthz`.

What the chart does NOT deploy (intentionally — variability across
clusters is too high to template usefully):

- Postgres — customer brings their own
- Ingress / Gateway / LoadBalancer — wire your own to the Service
- HorizontalPodAutoscaler — customer-specific scaling policy
- ServiceMonitor / PodMonitor — customer-specific observability stack
- PersistentVolumeClaim for audit archive — set `blob.auditArchiveDir`
  to a path backed by your PVC mount when you want F5.3-c-ii-archive
  cold-archive enabled

## Values reference

See `values.yaml` for the full schema and defaults. Common overrides:

| Path | Purpose | Default |
|------|---------|---------|
| `replicaCount` | Number of cloud-serve pods | 1 |
| `image.repository` | Container image | `ghcr.io/ministr-ai/cloud` |
| `image.tag` | Image tag | `.Chart.AppVersion` |
| `license.key` | F5.4-a license JWT | **required** |
| `license.publicKey` | F5.4-a verification key | **required** |
| `postgres.url` | DB connection string | **required** |
| `blob.azureAccount` | Azure Blob account for HNSW bundles | (none — uses ephemeral) |
| `blob.azureContainer` | Azure Blob container name | (none) |
| `blob.auditArchiveDir` | F5.3-c-ii-archive FS path | (none — endpoint 503s) |
| `service.type` | k8s Service type | `ClusterIP` |
| `service.port` | Service port | 8080 |
| `resources.limits.cpu` | CPU limit | `2000m` |
| `resources.limits.memory` | Memory limit | `2Gi` |
| `extraEnv` | Map of additional env vars | `{}` |

## Upgrade

```bash
helm upgrade ministr-enterprise ./deploy/helm/ministr-enterprise \
  --namespace ministr -f values.local.yaml
```

The Deployment rolling-updates with default strategy. Cloud serve boots
honor the license gate (F5.4-a) at every start; an expired key will
keep new pods in CrashLoopBackOff until you rotate.

## Uninstall

```bash
helm uninstall ministr-enterprise --namespace ministr
```

This removes the Deployment, Service, and Secret. Your Postgres data
(orgs, members, audit_events) is untouched — the customer-supplied
database is outside the chart's scope.

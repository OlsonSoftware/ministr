# ministr-enterprise Docker Compose template

F5.4-d — single-node trial / dev deployment of ministr-cloud.
Bundles a Postgres instance for ease of setup; production deployments
override to use their own external DB.

## Prerequisites

- Docker Engine 24+ with the Compose v2 plugin (`docker compose`, not
  `docker-compose`)
- A ministr Enterprise license (key JWT + RS256 public key) — request
  from `support@ministr.ai`

## Quick start

```bash
cd deploy/compose
cp .env.example .env
# Edit .env — paste your MINISTR_LICENSE_KEY, MINISTR_LICENSE_PUBLIC_KEY,
# and set POSTGRES_PASSWORD to a strong random value.

docker compose -f ministr-enterprise.yml up -d
```

This brings up two containers:

- `ministr-postgres` (Postgres 16, alpine) — bundled DB on a named
  volume `ministr-pg-data`.
- `ministr-cloud` (the serve binary) — depends on `ministr-postgres`
  via `service_healthy` so the first connection attempt isn't a
  noisy retry loop.

Both containers register healthchecks. The compose file's
`POSTGRES_PASSWORD:?…` / `MINISTR_LICENSE_KEY:?…` style means
`docker compose up` errors loudly if any required env var is unset
— no half-deployed stack.

## Verify

```bash
# Tail logs — look for "Enterprise license validated"
docker compose -f ministr-enterprise.yml logs -f cloud

# Hit /healthz
curl http://localhost:8080/healthz
```

## Production (external Postgres)

Bundled Postgres is fine for trials but not production-recommended
(no backups, no point-in-time recovery, no monitoring). For
production:

1. Set `MINISTR_PG_URL=postgres://user:pass@your-db.example.com:5432/ministr?sslmode=require`
   in your `.env`.
2. Remove the `postgres` service block from `ministr-enterprise.yml`
   (or start only the cloud service: `docker compose up -d cloud`).
3. Remove the `depends_on: postgres:` block on the `cloud` service.
4. Remove the `ministr-pg-data` named volume.

## Stop / cleanup

```bash
# Stop and remove containers (preserves volumes)
docker compose -f ministr-enterprise.yml down

# Stop + remove containers + DELETE volumes (destroys your data)
docker compose -f ministr-enterprise.yml down --volumes
```

## What this template does NOT do

- **TLS termination** — wire your own reverse proxy (nginx, Caddy,
  Traefik) in front of port 8080. The cloud serve speaks HTTP only;
  TLS is the deployment's job.
- **Multi-pod scaling** — single replica only. Use the F5.4-c Helm
  chart on Kubernetes for that.
- **Backups** — your responsibility (pg_dump, point-in-time, etc).
- **Image build** — pulls from `ghcr.io/ministr-ai/cloud:0.6.0` by
  default. Override `MINISTR_IMAGE` if running behind a firewall or
  pinning a specific build.

## When to use this vs the Helm chart (F5.4-c)

| Scenario | Use |
|----------|-----|
| Local dev / evaluating ministr | Docker Compose (this) |
| Single-node production trial (≤50 seats) | Docker Compose with external DB |
| Multi-node / HA / scaling | Helm chart (`deploy/helm/ministr-enterprise`) |
| Customer-managed K8s cluster | Helm chart |

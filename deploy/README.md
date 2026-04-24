# Deploying ministr as a Remote MCP Server

ministr supports Streamable HTTP transport, enabling remote deployment as an MCP server that any MCP client can connect to over HTTPS.

## Contents

- [Docker](#quick-start-docker) — build and run the image
- [Fly.io](#flyio) — serverless deploy with a mounted volume
- [Railway](#railway) — auto-deploy from the Dockerfile
- [Reverse proxy: nginx](#reverse-proxy-nginx) — TLS termination, SSE passthrough
- [Reverse proxy: Caddy](#reverse-proxy-caddy) — automatic TLS via ACME
- [Connecting MCP clients](#connecting-mcp-clients) — URL + OAuth config
- [Resource requirements](#resource-requirements) — RAM, CPU, storage

## Quick Start (Docker)

```bash
# Build the image
docker build -t ministr .

# Run with a corpus directory mounted
docker run -p 8080:8080 -v /path/to/code:/corpus -v ministr_data:/data \
  ministr --corpus /corpus

# With OAuth enabled
docker run -p 8080:8080 -v /path/to/code:/corpus -v ministr_data:/data \
  ministr --corpus /corpus --oauth --oauth-issuer https://ministr.example.com
```

The `/data` volume persists the index between restarts, avoiding re-ingestion.

## Fly.io

ministr includes a `fly.toml` configured for Fly.io deployment:

```bash
# Install flyctl
curl -L https://fly.io/install.sh | sh

# Launch (first time)
fly launch --no-deploy
fly volumes create ministr_data --size 10 --region iad
fly deploy

# Push corpus to the volume, then configure
fly ssh console
# Inside the machine:
ministr index --corpus /data/your-repo
```

Requirements:
- **Memory**: 2 GB minimum (embedding model + HNSW index)
- **Storage**: 10 GB volume recommended for large codebases
- **Cold start**: ~5-10s when scaling from zero (model loading)

## Railway

Railway auto-detects the Dockerfile:

```bash
# Install Railway CLI
npm install -g @railway/cli

# Deploy
railway init
railway up
```

Railway sets the `PORT` environment variable automatically. The start command in `deploy/railway.toml` references it.

## Reverse Proxy (nginx)

If running ministr on a VPS, use `deploy/nginx.conf` for TLS termination:

```bash
# Copy and customize the config
sudo cp deploy/nginx.conf /etc/nginx/sites-available/ministr
sudo sed -i 's/ministr.example.com/your-domain.com/g' /etc/nginx/sites-available/ministr
sudo ln -s /etc/nginx/sites-available/ministr /etc/nginx/sites-enabled/
sudo certbot --nginx -d your-domain.com
sudo nginx -t && sudo systemctl reload nginx
```

Key settings:
- `proxy_buffering off` — required for SSE streaming
- `proxy_read_timeout 300s` — MCP sessions can be long-lived
- `Connection ""` — prevents connection: close on keep-alive

## Reverse Proxy (Caddy)

Caddy handles TLS automatically via ACME:

```bash
# Copy and customize
cp deploy/Caddyfile /etc/caddy/Caddyfile
sed -i 's/ministr.example.com/your-domain.com/g' /etc/caddy/Caddyfile
sudo systemctl reload caddy
```

Key setting: `flush_interval -1` disables response buffering for SSE.

## Connecting MCP Clients

Once deployed, configure your MCP client to connect via Streamable HTTP:

```json
{
  "mcpServers": {
    "ministr": {
      "url": "https://ministr.example.com/mcp"
    }
  }
}
```

With OAuth:
```json
{
  "mcpServers": {
    "ministr": {
      "url": "https://ministr.example.com/mcp",
      "auth": {
        "type": "oauth2",
        "authorization_url": "https://ministr.example.com/oauth/authorize",
        "token_url": "https://ministr.example.com/oauth/token",
        "registration_url": "https://ministr.example.com/oauth/register"
      }
    }
  }
}
```

## Resource Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| RAM | 2 GB | 4 GB |
| CPU | 1 vCPU | 2 vCPU |
| Storage | 5 GB | 10 GB |
| Network | HTTPS | HTTPS + WSS |

The embedding model (~80 MB) is downloaded on first startup. Subsequent starts use the cached model from the data volume.

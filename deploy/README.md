# Deploying iris as a Remote MCP Server

iris supports Streamable HTTP transport, enabling remote deployment as an MCP server that any MCP client can connect to over HTTPS.

## Quick Start (Docker)

```bash
# Build the image
docker build -t iris .

# Run with a corpus directory mounted
docker run -p 8080:8080 -v /path/to/code:/corpus -v iris_data:/data \
  iris --corpus /corpus

# With OAuth enabled
docker run -p 8080:8080 -v /path/to/code:/corpus -v iris_data:/data \
  iris --corpus /corpus --oauth --oauth-issuer https://iris.example.com
```

The `/data` volume persists the index between restarts, avoiding re-ingestion.

## Fly.io

iris includes a `fly.toml` configured for Fly.io deployment:

```bash
# Install flyctl
curl -L https://fly.io/install.sh | sh

# Launch (first time)
fly launch --no-deploy
fly volumes create iris_data --size 10 --region iad
fly deploy

# Push corpus to the volume, then configure
fly ssh console
# Inside the machine:
iris index --corpus /data/your-repo
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

If running iris on a VPS, use `deploy/nginx.conf` for TLS termination:

```bash
# Copy and customize the config
sudo cp deploy/nginx.conf /etc/nginx/sites-available/iris
sudo sed -i 's/iris.example.com/your-domain.com/g' /etc/nginx/sites-available/iris
sudo ln -s /etc/nginx/sites-available/iris /etc/nginx/sites-enabled/
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
sed -i 's/iris.example.com/your-domain.com/g' /etc/caddy/Caddyfile
sudo systemctl reload caddy
```

Key setting: `flush_interval -1` disables response buffering for SSE.

## Connecting MCP Clients

Once deployed, configure your MCP client to connect via Streamable HTTP:

```json
{
  "mcpServers": {
    "iris": {
      "url": "https://iris.example.com/mcp"
    }
  }
}
```

With OAuth:
```json
{
  "mcpServers": {
    "iris": {
      "url": "https://iris.example.com/mcp",
      "auth": {
        "type": "oauth2",
        "authorization_url": "https://iris.example.com/oauth/authorize",
        "token_url": "https://iris.example.com/oauth/token",
        "registration_url": "https://iris.example.com/oauth/register"
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

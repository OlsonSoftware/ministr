# ministr-release-proxy

Cloudflare Worker that lets `curl https://dl.ministr.app/<tag>/<filename>`
work even though the source repo is private. The Worker sits in front of
the GitHub release-asset API with a read-only fine-grained PAT and forwards
unauthenticated clients onto GitHub's pre-signed CDN URLs.

## Endpoints

| Path | Response |
|---|---|
| `GET /` | Short usage banner. |
| `GET /latest` | JSON metadata: `tag`, `name`, `published_at`, `html_url`, `assets[]`. Cached 5 min at the edge. |
| `GET /latest/<filename>` | 302 to the asset bytes for the latest release. |
| `GET /<tag>/<filename>` | 302 to the asset bytes for an explicit tag (e.g. `/v0.1.0/ministr-x86_64-apple-darwin.tar.gz`). |

The 302 points to GitHub's short-lived (≈5 min) CDN URL, so Worker
CPU/wall-time stays trivial regardless of binary size.

## Deploying

Requires `wrangler` logged into the right Cloudflare account.

```sh
cd workers/release-proxy
wrangler secret put GITHUB_TOKEN         # paste a fine-grained PAT with Contents:Read on OlsonSoftware/ministr
wrangler deploy
```

The `wrangler.toml` declares `dl.ministr.app` as a custom domain — on first
deploy wrangler creates the DNS record and provisions the cert
automatically. No Redirect Rule interaction: the `ministr.app` apex
redirect ruleset is scoped by `http.host`, so `dl.*` is untouched.

## Rotating the token

```sh
wrangler secret put GITHUB_TOKEN         # paste new token, old one is replaced
```

Workers pull the secret at cold start, so the old token is flushed within
a minute.

## Flipping the source repo public later

If `OlsonSoftware/ministr` becomes public, delete the Worker and point
`install.sh` directly at
`https://github.com/OlsonSoftware/ministr/releases/download/<tag>/<file>`
— one file changes.

## Local testing

```sh
wrangler dev
# then
curl http://localhost:8787/latest
```

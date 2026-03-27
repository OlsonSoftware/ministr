# iris cloud — Technical Plan

The paid service that makes iris instant for dependencies.

## The Pitch

iris is free. iris cloud makes it fast.

Instead of cloning + indexing `tokio` (3000+ files, minutes of ONNX inference), you write:

```toml
[[corpus.cloud]]
repo = "tokio-rs/tokio"
paths = ["tokio/src"]
```

iris downloads the pre-computed index in seconds. No clone, no parsing, no embedding.

## Architecture

```
Developer's machine                    iris cloud
┌──────────────┐                   ┌─────────────────┐
│  iris CLI     │  ← HTTP GET →    │  Index API       │
│  (free, OSS)  │  .iris-index     │  (pre-computed)  │
│               │  bundle          │                  │
│  SQLite       │                  │  Index Worker    │
│  HNSW Index   │                  │  (clones repos,  │
│  Embeddings   │                  │   indexes daily)  │
└──────────────┘                   └─────────────────┘
```

### Index Bundle Format (.iris-index)

A compressed archive containing everything iris needs to load a corpus root:

```
bundle.iris-index (gzip or zstd compressed)
├── meta.toml              # repo URL, commit SHA, timestamp, model name, paths
├── sections.jsonl         # all sections with heading paths, summaries, claims
├── symbols.jsonl          # all symbols with signatures, doc comments, module paths
├── refs.jsonl             # symbol references (imports, implements, calls, uses)
├── bridge_endpoints.jsonl # bridge export/import endpoints
├── bridge_links.jsonl     # matched bridge links
├── embeddings.bin         # dense vectors (f32 little-endian, section-ordered)
├── embedding_index.jsonl  # maps vector position → content ID
└── file_hashes.jsonl      # content hashes for incremental updates
```

Key design decisions:
- **JSONL** for structured data (streamable, debuggable, versionable)
- **Binary** for embeddings only (dense f32 vectors don't compress well as JSON)
- **Model name in meta** — bundles are tied to a specific embedding model. iris validates on import.
- **No HNSW graph** — rebuilt locally from embeddings on import (fast, avoids platform-specific binary issues)

### Cloud API

Simple REST API. No auth for public repos. API key for private repos + higher rate limits.

```
GET /v1/index/{owner}/{repo}
    ?paths=src,docs
    &branch=main
    &model=all-MiniLM-L6-v2

Response: 200 OK
Content-Type: application/octet-stream
X-Iris-Commit: abc123
X-Iris-Indexed-At: 2026-03-23T10:00:00Z
X-Iris-Sections: 2646
X-Iris-Symbols: 1200

[binary .iris-index bundle]
```

```
HEAD /v1/index/{owner}/{repo}   # check freshness without downloading
    → X-Iris-Commit: abc123     # compare with local cached version
```

### Index Worker

A background service that:
1. Maintains a list of "popular" repos (top N by download count or user request)
2. Clones each repo on a schedule (daily or on webhook)
3. Runs iris indexing pipeline (parse → extract → embed → bundle)
4. Stores bundles in S3/R2 (Cloudflare R2 for cheap egress)
5. Serves bundles via CDN

Stack options (cheapest to most scalable):
- **MVP**: Single VPS (Hetzner, $20/mo) + R2 storage + Cloudflare CDN
- **Scale**: Fly.io workers + R2 + Cloudflare CDN
- **Enterprise**: Kubernetes + S3 + CloudFront

### Pricing

| Tier | Price | What you get |
|------|-------|-------------|
| Free | $0 | iris CLI (all features), local indexing, `iris_clone` |
| Cloud | $15/mo | Pre-indexed repos (top 500), instant import, auto-refresh |
| Team | $25/user/mo | Shared team corpus, SSO, private repo indexing |

Revenue model: usage is light per user (a few index downloads per month), so margins are high. A $20/mo VPS can serve thousands of users.

## Implementation Phases

### Phase 0: Export/Import (in iris-rs repo)

This is the foundation — build it before any cloud infra.

1. Design the `.iris-index` bundle format (meta.toml + JSONL + binary embeddings)
2. `iris export --root <root-id> -o bundle.iris-index` — dumps a corpus root to a bundle
3. `iris import bundle.iris-index` — loads a bundle into local SQLite + HNSW
4. `[[corpus.cloud]]` in `.iris.toml` — fetches from a URL and imports
5. Versioning: compare local commit SHA with remote, re-fetch if stale

This phase is pure Rust, lives in the iris-rs repo, and can ship as an open source feature even before the cloud service exists. Users can share `.iris-index` files manually (post them as GitHub release assets, etc.).

### Phase 1: Cloud MVP (separate repo — `iris-cloud`)

1. Simple Rust HTTP server (axum) that serves `.iris-index` bundles from disk/R2
2. Index worker: cron job that clones top 100 repos, runs iris indexing, uploads bundles
3. R2 storage + Cloudflare CDN for serving bundles
4. Simple API key auth via Stripe for $15/mo tier
5. Landing page with "iris cloud" branding and Stripe checkout

Tech stack:
- **API server**: Rust (axum) on Fly.io — same language as iris, share iris-core as a dependency
- **Storage**: Cloudflare R2 ($0.015/GB/mo, free egress via CDN)
- **Auth/billing**: Stripe Checkout + API keys
- **Worker**: Fly.io machine that runs on a schedule
- **Domain**: iriscloud.dev or iris.rs

### Phase 2: Scale

1. User-requested repos: "request an index" button on the landing page
2. Webhook-triggered re-indexing (GitHub webhooks on push to default branch)
3. Private repo support (user provides GitHub token, we index in an ephemeral container)
4. Team features: shared corpus config, SSO via WorkOS

## Cost Estimates (MVP)

| Item | Monthly Cost |
|------|-------------|
| Fly.io API server (shared-cpu-1x) | $5 |
| Fly.io worker (dedicated-cpu-2x, runs 2h/day) | $15 |
| Cloudflare R2 (100GB storage) | $1.50 |
| Cloudflare CDN | Free |
| Domain | $1 |
| Stripe | 2.9% + $0.30 per transaction |
| **Total** | **~$23/mo** |

Break-even: 2 subscribers at $15/mo.

## Timeline

| When | What |
|------|------|
| Week 1-2 | Ship `iris export` / `iris import` in iris-rs |
| Week 3 | Set up iris-cloud repo, axum API, R2 storage |
| Week 4 | Index worker: auto-index top 50 repos |
| Month 2 | Launch cloud waitlist, onboard first 10 beta users |
| Month 3 | Public launch at $15/mo, target 50 subscribers |
| Month 6 | 200 subscribers = $3k MRR |

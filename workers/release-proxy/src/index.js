// Release proxy for ministr.
//
// The source repo (OlsonSoftware/ministr) is private. We still want
// unauthenticated users to be able to `curl https://dl.ministr.app/<tag>/<file>`
// and pull down a release binary. This Worker sits in front of GitHub's
// release-asset API with a read-only fine-grained PAT and streams binaries
// back to the caller.
//
// Endpoints
//   GET /                             — short usage banner
//   GET /latest                       — JSON: tag + asset list for the latest release
//   GET /latest/<filename>            — 302 to the asset bytes for the latest release
//   GET /<tag>/<filename>             — 302 to the asset bytes for an explicit tag
//
// Secret binding
//   GITHUB_TOKEN — fine-grained PAT with Contents:Read on OlsonSoftware/ministr
//
// Cost protection
//   Two cache layers sit on top of every GitHub API call:
//   1. Response cache (full-request URL) — 2 min for 302 redirects, 5 min for
//      /latest metadata. Serves repeat downloads of the same file straight
//      from the CF edge with 0 GH API calls and minimal Worker compute.
//   2. Metadata cache (per-tag JSON) — 7 days for immutable tags, 5 min for
//      `latest`. Layer 1 misses still skip the release-lookup call when the
//      tag is already known.
//   Negative responses (404/400) cache for 1 min so garbage scanners can't
//   hammer the GitHub API.
//   Path validation rejects requests before any upstream call is made.

const DEFAULT_REPO = 'OlsonSoftware/ministr';
const LATEST = 'latest';

// Accept only sane tag and filename shapes. Blocks path-traversal probes and
// general bot noise (`/wp-admin`, `/.env`, …) before any API call fires.
const TAG_RE = /^[vV]?[\w.+-]{1,100}$/;
const FILENAME_RE = /^[\w.+-]{1,200}$/;

// Cache TTLs (seconds).
const METADATA_TAG_TTL = 7 * 24 * 60 * 60; // 7d — tags are immutable once cut
const METADATA_LATEST_TTL = 300;           // 5m — "latest" can change when a new release lands
const REDIRECT_TTL = 120;                  // 2m — below the ~5m lifetime of GitHub's pre-signed CDN URLs
const NEGATIVE_TTL = 60;                   // 1m — don't let 404s flood us with GH calls
const BANNER_TTL = 3600;                   // 1h — root is static text

function ghHeaders(token) {
  return {
    'Accept': 'application/vnd.github+json',
    'Authorization': `Bearer ${token}`,
    'User-Agent': 'ministr-release-proxy',
    'X-GitHub-Api-Version': '2022-11-28',
  };
}

// Internal cache key for per-tag metadata. Uses `.invalid` TLD so it cannot
// ever collide with a real request URL.
function metaKey(repo, tag) {
  return new Request(`https://cache.internal.invalid/meta/${encodeURIComponent(repo)}/${encodeURIComponent(tag)}`);
}

// Fetch + cache the release JSON for a given tag (or "latest"). Returns
// { ok, release } where `release` is the parsed GitHub response (or the
// minimal negative-cache stub on miss). Uses ctx.waitUntil so the cache
// write doesn't block the response.
async function getRelease(repo, tag, token, ctx) {
  const cache = caches.default;
  const key = metaKey(repo, tag);

  const cached = await cache.match(key);
  if (cached) {
    try {
      const body = await cached.text();
      if (!cached.ok) return { ok: false };
      return { ok: true, release: JSON.parse(body) };
    } catch {
      // Corrupt cache entry — fall through to a fresh fetch.
    }
  }

  const api = tag === LATEST
    ? `https://api.github.com/repos/${repo}/releases/latest`
    : `https://api.github.com/repos/${repo}/releases/tags/${encodeURIComponent(tag)}`;
  const resp = await fetch(api, { headers: ghHeaders(token) });
  const body = await resp.text();

  const ttl = resp.ok
    ? (tag === LATEST ? METADATA_LATEST_TTL : METADATA_TAG_TTL)
    : NEGATIVE_TTL;

  ctx.waitUntil(
    cache.put(
      key,
      new Response(body, {
        status: resp.status,
        headers: {
          'Content-Type': 'application/json',
          'Cache-Control': `public, max-age=${ttl}`,
        },
      }),
    ),
  );

  if (!resp.ok) return { ok: false };
  try {
    return { ok: true, release: JSON.parse(body) };
  } catch {
    return { ok: false };
  }
}

function cachedError(body, status, ttl) {
  return new Response(body, {
    status,
    headers: {
      'Content-Type': 'text/plain; charset=utf-8',
      'Cache-Control': `public, max-age=${ttl}`,
    },
  });
}

function banner() {
  return new Response(
    [
      'ministr release proxy',
      '',
      'GET /latest              → JSON metadata for the latest release',
      'GET /latest/<filename>   → 302 to asset bytes (latest release)',
      'GET /<tag>/<filename>    → 302 to asset bytes (explicit tag, e.g. v0.1.0)',
      '',
    ].join('\n'),
    {
      headers: {
        'Content-Type': 'text/plain; charset=utf-8',
        'Cache-Control': `public, max-age=${BANNER_TTL}`,
      },
    },
  );
}

export default {
  async fetch(req, env, ctx) {
    if (req.method !== 'GET' && req.method !== 'HEAD') {
      return new Response('method not allowed', {
        status: 405,
        headers: { 'Cache-Control': `public, max-age=${NEGATIVE_TTL}` },
      });
    }

    const token = env.GITHUB_TOKEN;
    if (!token) {
      return new Response('server misconfigured: GITHUB_TOKEN unbound', { status: 500 });
    }
    const repo = env.GITHUB_REPO || DEFAULT_REPO;

    const url = new URL(req.url);
    const cache = caches.default;

    // Outer cache: keyed by full request URL. Serves repeat identical
    // requests directly from the edge with zero GH API cost.
    const outerKey = new Request(req.url, { method: 'GET' });
    const cachedOuter = await cache.match(outerKey);
    if (cachedOuter) return cachedOuter;

    const parts = url.pathname.split('/').filter(Boolean);

    if (parts.length === 0) {
      const resp = banner();
      ctx.waitUntil(cache.put(outerKey, resp.clone()));
      return resp;
    }

    // /latest → metadata JSON
    if (parts.length === 1 && parts[0] === LATEST) {
      const { ok, release } = await getRelease(repo, LATEST, token, ctx);
      if (!ok) return cachedError('no releases yet', 404, NEGATIVE_TTL);

      const body = {
        tag: release.tag_name,
        name: release.name,
        published_at: release.published_at,
        html_url: release.html_url,
        assets: release.assets.map((a) => ({
          name: a.name,
          size: a.size,
          content_type: a.content_type,
        })),
      };
      const resp = new Response(JSON.stringify(body, null, 2), {
        headers: {
          'Content-Type': 'application/json; charset=utf-8',
          'Cache-Control': `public, max-age=${METADATA_LATEST_TTL}`,
        },
      });
      ctx.waitUntil(cache.put(outerKey, resp.clone()));
      return resp;
    }

    if (parts.length !== 2) {
      return cachedError('not found — try /latest or /<tag>/<filename>', 404, NEGATIVE_TTL);
    }

    const [tag, filename] = parts;

    // Cheap path-shape rejection — never hits GH for obvious garbage.
    if (tag !== LATEST && !TAG_RE.test(tag)) {
      return cachedError('bad tag', 400, NEGATIVE_TTL);
    }
    if (!FILENAME_RE.test(filename)) {
      return cachedError('bad filename', 400, NEGATIVE_TTL);
    }

    const { ok, release } = await getRelease(repo, tag, token, ctx);
    if (!ok) {
      return cachedError(`release not found: ${tag}`, 404, NEGATIVE_TTL);
    }
    const asset = release.assets.find((a) => a.name === filename);
    if (!asset) {
      return cachedError(`asset not found in ${release.tag_name}: ${filename}`, 404, NEGATIVE_TTL);
    }

    // Request the asset with Accept: octet-stream. GitHub returns a 302 to a
    // short-lived pre-signed CDN URL. We forward that URL so the client
    // downloads directly from GitHub's CDN — zero bytes flow through the
    // Worker, so CPU and egress stay trivial regardless of binary size.
    const assetResp = await fetch(
      `https://api.github.com/repos/${repo}/releases/assets/${asset.id}`,
      {
        headers: { ...ghHeaders(token), 'Accept': 'application/octet-stream' },
        redirect: 'manual',
      },
    );

    if (assetResp.status >= 300 && assetResp.status < 400) {
      const location = assetResp.headers.get('Location');
      if (!location) return new Response('upstream redirect without Location', { status: 502 });
      const resp = new Response(null, {
        status: 302,
        headers: {
          'Location': location,
          'Cache-Control': `public, max-age=${REDIRECT_TTL}`,
        },
      });
      ctx.waitUntil(cache.put(outerKey, resp.clone()));
      return resp;
    }

    // Upstream returned the body inline (small assets, rare). Stream it
    // through without caching — large bodies would blow the free-tier cache
    // entry size limit and we already covered the redirect path above.
    if (assetResp.ok) {
      return new Response(assetResp.body, {
        status: 200,
        headers: {
          'Content-Type': asset.content_type || 'application/octet-stream',
          'Content-Disposition': `attachment; filename="${filename}"`,
          'Content-Length': String(asset.size),
        },
      });
    }

    return cachedError(`upstream ${assetResp.status}`, 502, NEGATIVE_TTL);
  },
};

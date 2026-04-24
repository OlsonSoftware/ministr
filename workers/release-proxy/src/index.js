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
// Config (vars, not secrets)
//   GITHUB_REPO  — defaults to "OlsonSoftware/ministr"

const DEFAULT_REPO = 'OlsonSoftware/ministr';

function ghHeaders(token) {
  return {
    'Accept': 'application/vnd.github+json',
    'Authorization': `Bearer ${token}`,
    'User-Agent': 'ministr-release-proxy',
    'X-GitHub-Api-Version': '2022-11-28',
  };
}

async function getRelease(repo, tag, token) {
  const api =
    tag === 'latest'
      ? `https://api.github.com/repos/${repo}/releases/latest`
      : `https://api.github.com/repos/${repo}/releases/tags/${encodeURIComponent(tag)}`;
  const resp = await fetch(api, { headers: ghHeaders(token) });
  if (!resp.ok) return null;
  return resp.json();
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
    { headers: { 'Content-Type': 'text/plain; charset=utf-8' } },
  );
}

export default {
  async fetch(req, env) {
    if (req.method !== 'GET' && req.method !== 'HEAD') {
      return new Response('method not allowed', { status: 405 });
    }

    const token = env.GITHUB_TOKEN;
    if (!token) {
      return new Response('server misconfigured: GITHUB_TOKEN unbound', { status: 500 });
    }
    const repo = env.GITHUB_REPO || DEFAULT_REPO;

    const url = new URL(req.url);
    const parts = url.pathname.split('/').filter(Boolean);

    if (parts.length === 0) {
      return banner();
    }

    // /latest → metadata JSON
    if (parts.length === 1 && parts[0] === 'latest') {
      const rel = await getRelease(repo, 'latest', token);
      if (!rel) {
        return new Response('no releases yet', { status: 404 });
      }
      const body = {
        tag: rel.tag_name,
        name: rel.name,
        published_at: rel.published_at,
        html_url: rel.html_url,
        assets: rel.assets.map((a) => ({
          name: a.name,
          size: a.size,
          content_type: a.content_type,
        })),
      };
      return new Response(JSON.stringify(body, null, 2), {
        headers: {
          'Content-Type': 'application/json; charset=utf-8',
          // Short cache so a new release shows up within 5 min without stampede.
          'Cache-Control': 'public, max-age=300',
        },
      });
    }

    if (parts.length !== 2) {
      return new Response('not found — try /latest or /<tag>/<filename>', { status: 404 });
    }

    const [tag, filename] = parts;
    const rel = await getRelease(repo, tag, token);
    if (!rel) {
      return new Response(`release not found: ${tag}`, { status: 404 });
    }
    const asset = rel.assets.find((a) => a.name === filename);
    if (!asset) {
      return new Response(`asset not found in ${rel.tag_name}: ${filename}`, { status: 404 });
    }

    // Request the asset with Accept: octet-stream. GitHub returns a 302 to a
    // short-lived pre-signed CDN URL — we forward the Location header so the
    // client downloads directly from GitHub's CDN, not through the Worker.
    // That keeps Worker CPU/wall-time well under limits for large binaries.
    const assetResp = await fetch(
      `https://api.github.com/repos/${repo}/releases/assets/${asset.id}`,
      {
        headers: { ...ghHeaders(token), 'Accept': 'application/octet-stream' },
        redirect: 'manual',
      },
    );

    if (assetResp.status >= 300 && assetResp.status < 400) {
      const location = assetResp.headers.get('Location');
      if (!location) {
        return new Response('upstream redirect without Location', { status: 502 });
      }
      return Response.redirect(location, 302);
    }

    // Fallback: upstream returned body inline (some tokens, smaller assets).
    // Stream it through with Content-Disposition so curl -O works sensibly.
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

    return new Response(`upstream ${assetResp.status}`, { status: 502 });
  },
};

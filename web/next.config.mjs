// GitHub Pages now serves this site from https://ministr.ai/ at the root
// via the `web/public/CNAME` file. `DOCS_BASE_PATH` stays available
// so a developer can still preview against a sub-path deployment
// (e.g. `/ministr/`) by exporting the env var locally, but production
// builds run with an empty basePath.
const basePath = process.env.DOCS_BASE_PATH ?? '';

/** @type {import('next').NextConfig} */
const config = {
  output: 'export',
  reactStrictMode: true,
  basePath,
  trailingSlash: true,
  images: { unoptimized: true },
  env: {
    NEXT_PUBLIC_BASE_PATH: basePath,
  },
  // Enable native cross-document View Transitions through Next.js soft navs.
  // Requires Next.js 15+; falls back to the CSS @view-transition rule for
  // hard document loads in supported browsers.
  experimental: {
    viewTransition: true,
  },
};

export default config;

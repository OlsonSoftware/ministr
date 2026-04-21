import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

// GitHub Pages serves this site from https://AlrikOlson.github.io/ministr-rs/
// so all asset URLs and internal links need a /ministr-rs prefix. The
// DOCS_BASE_PATH env var is set by the GH workflow; local dev runs at /.
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

export default withMDX(config);

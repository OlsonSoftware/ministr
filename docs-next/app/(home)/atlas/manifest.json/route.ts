// F2.6 — `/atlas/manifest.json` public mirror.
//
// This is the public-transparency endpoint third parties scrape to
// know what Atlas indexes. Same shape as the cloud's
// `GET /atlas/manifest.json` route handler in `ministr-atlas` — but
// served by docs-next so unauthenticated browsers can fetch it
// without crossing the `mcp.ministr.ai` auth boundary.
//
// Source of truth: `ministr-atlas/src/repos.rs::ATLAS_SEED_REPOS`.
// This file mirrors that data by hand; CI keeps them in sync by
// invoking `ministr atlas manifest` and diffing against this file.
// When the Rust list changes, regenerate with:
//
//     ministr atlas manifest
//
// and paste the entries into the `ATLAS` array below. Future-work:
// a prebuild script that runs the Rust CLI and writes this file
// automatically.
//
// Static-export safe: `dynamic = 'force-static'` makes Next.js
// pre-render the route at build time. No per-request rendering;
// fully cacheable on a CDN.

import { NextResponse } from 'next/server';

export const dynamic = 'force-static';

interface ManifestEntry {
  slug: string;
  clone_url: string;
  spdx: string;
  description: string;
  opted_out: boolean;
}

interface ManifestSnapshot {
  schema_version: 1;
  count: number;
  entries: ManifestEntry[];
}

// Mirror of `ministr-atlas/src/repos.rs::ATLAS_SEED_REPOS`. Keep in
// sync by running `ministr atlas manifest`.
const ATLAS: ReadonlyArray<Omit<ManifestEntry, 'opted_out'>> = [
  { slug: 'react', clone_url: 'https://github.com/facebook/react', spdx: 'MIT', description: 'The React UI library — the most-queried JS codebase on GitHub.' },
  { slug: 'vue', clone_url: 'https://github.com/vuejs/core', spdx: 'MIT', description: 'Vue 3 core — reactive framework with a runtime + compiler split.' },
  { slug: 'svelte', clone_url: 'https://github.com/sveltejs/svelte', spdx: 'MIT', description: 'Svelte 5 — runes-based compiler and component model.' },
  { slug: 'nextjs', clone_url: 'https://github.com/vercel/next.js', spdx: 'MIT', description: 'Next.js — React meta-framework with App Router and Turbopack.' },
  { slug: 'tailwindcss', clone_url: 'https://github.com/tailwindlabs/tailwindcss', spdx: 'MIT', description: 'Tailwind CSS — utility-first CSS framework, v4 engine.' },
  { slug: 'remix', clone_url: 'https://github.com/remix-run/remix', spdx: 'MIT', description: 'Remix — React framework now merged with React Router.' },
  { slug: 'astro', clone_url: 'https://github.com/withastro/astro', spdx: 'MIT', description: 'Astro — content-driven web framework with islands architecture.' },
  { slug: 'solid', clone_url: 'https://github.com/solidjs/solid', spdx: 'MIT', description: 'SolidJS — fine-grained reactive primitives, JSX without VDOM.' },
  { slug: 'django', clone_url: 'https://github.com/django/django', spdx: 'BSD-3-Clause', description: 'Django — the batteries-included Python web framework.' },
  { slug: 'fastapi', clone_url: 'https://github.com/tiangolo/fastapi', spdx: 'MIT', description: 'FastAPI — modern Python web framework on Starlette + Pydantic.' },
  { slug: 'flask', clone_url: 'https://github.com/pallets/flask', spdx: 'BSD-3-Clause', description: 'Flask — the original micro web framework for Python.' },
  { slug: 'rails', clone_url: 'https://github.com/rails/rails', spdx: 'MIT', description: 'Ruby on Rails — the original convention-over-configuration web stack.' },
  { slug: 'spring-boot', clone_url: 'https://github.com/spring-projects/spring-boot', spdx: 'Apache-2.0', description: 'Spring Boot — opinionated stand-alone Spring application bootstrapper.' },
  { slug: 'gin', clone_url: 'https://github.com/gin-gonic/gin', spdx: 'MIT', description: 'Gin — high-performance HTTP framework for Go.' },
  { slug: 'echo', clone_url: 'https://github.com/labstack/echo', spdx: 'MIT', description: 'Echo — high-performance Go web framework with middleware-first design.' },
  { slug: 'postgres', clone_url: 'https://github.com/postgres/postgres', spdx: 'PostgreSQL', description: 'PostgreSQL — the canonical open-source relational database.' },
  { slug: 'sqlite', clone_url: 'https://github.com/sqlite/sqlite', spdx: 'blessing', description: 'SQLite — public-domain embedded SQL database engine.' },
  { slug: 'redis', clone_url: 'https://github.com/redis/redis', spdx: 'BSD-3-Clause', description: 'Redis — in-memory data structure store. Pre-licence-relicense fork lineage.' },
  { slug: 'valkey', clone_url: 'https://github.com/valkey-io/valkey', spdx: 'BSD-3-Clause', description: 'Valkey — Linux Foundation Redis fork, post-2024 relicensing.' },
  { slug: 'duckdb', clone_url: 'https://github.com/duckdb/duckdb', spdx: 'MIT', description: 'DuckDB — embedded analytical database, SQLite for OLAP.' },
  { slug: 'tokio', clone_url: 'https://github.com/tokio-rs/tokio', spdx: 'MIT', description: 'Tokio — async runtime + ecosystem for Rust.' },
  { slug: 'axum', clone_url: 'https://github.com/tokio-rs/axum', spdx: 'MIT', description: 'Axum — ergonomic, modular web framework on Tokio.' },
  { slug: 'serde', clone_url: 'https://github.com/serde-rs/serde', spdx: 'MIT', description: 'Serde — generic serialisation framework, the Rust ecosystem keystone.' },
  { slug: 'tracing', clone_url: 'https://github.com/tokio-rs/tracing', spdx: 'MIT', description: 'Tracing — structured async-aware diagnostics for Rust.' },
  { slug: 'rust', clone_url: 'https://github.com/rust-lang/rust', spdx: 'Apache-2.0', description: 'The Rust compiler and standard library.' },
  { slug: 'cargo', clone_url: 'https://github.com/rust-lang/cargo', spdx: 'Apache-2.0', description: "Cargo — Rust's package manager and build tool." },
  { slug: 'kubernetes', clone_url: 'https://github.com/kubernetes/kubernetes', spdx: 'Apache-2.0', description: 'Kubernetes — production-grade container orchestrator.' },
  { slug: 'containerd', clone_url: 'https://github.com/containerd/containerd', spdx: 'Apache-2.0', description: 'containerd — OCI container runtime.' },
  { slug: 'envoy', clone_url: 'https://github.com/envoyproxy/envoy', spdx: 'Apache-2.0', description: 'Envoy — high-performance L7 proxy and service-mesh data plane.' },
  { slug: 'prometheus', clone_url: 'https://github.com/prometheus/prometheus', spdx: 'Apache-2.0', description: 'Prometheus — pull-based monitoring + alerting system.' },
  { slug: 'grafana', clone_url: 'https://github.com/grafana/grafana', spdx: 'AGPL-3.0-only', description: 'Grafana — observability dashboards (AGPL — review under G.1 before retrieval).' },
  { slug: 'terraform-provider-aws', clone_url: 'https://github.com/hashicorp/terraform-provider-aws', spdx: 'MPL-2.0', description: 'Terraform AWS provider — broad coverage of AWS APIs.' },
  { slug: 'pytorch', clone_url: 'https://github.com/pytorch/pytorch', spdx: 'BSD-3-Clause', description: 'PyTorch — tensors and dynamic neural networks in Python with autograd.' },
  { slug: 'transformers', clone_url: 'https://github.com/huggingface/transformers', spdx: 'Apache-2.0', description: 'Hugging Face Transformers — pretrained model hub + training utilities.' },
  { slug: 'diffusers', clone_url: 'https://github.com/huggingface/diffusers', spdx: 'Apache-2.0', description: 'Hugging Face Diffusers — diffusion model pipelines.' },
  { slug: 'llama-cpp', clone_url: 'https://github.com/ggerganov/llama.cpp', spdx: 'MIT', description: 'llama.cpp — LLM inference in plain C/C++.' },
  { slug: 'ollama', clone_url: 'https://github.com/ollama/ollama', spdx: 'MIT', description: 'Ollama — run LLMs locally with a simple CLI.' },
  { slug: 'vite', clone_url: 'https://github.com/vitejs/vite', spdx: 'MIT', description: 'Vite — fast bundler/dev server built on Rollup + esbuild.' },
  { slug: 'esbuild', clone_url: 'https://github.com/evanw/esbuild', spdx: 'MIT', description: 'esbuild — extremely fast JavaScript/TypeScript bundler.' },
  { slug: 'rollup', clone_url: 'https://github.com/rollup/rollup', spdx: 'MIT', description: 'Rollup — ESM-first module bundler.' },
  { slug: 'biome', clone_url: 'https://github.com/biomejs/biome', spdx: 'MIT', description: 'Biome — formatter + linter for JS/TS/JSX/JSON/CSS, Rust-native.' },
  { slug: 'ruff', clone_url: 'https://github.com/astral-sh/ruff', spdx: 'MIT', description: 'Ruff — Rust-native Python linter and formatter, 10-100× faster than pylint.' },
  { slug: 'uv', clone_url: 'https://github.com/astral-sh/uv', spdx: 'MIT', description: 'uv — Rust-native Python package + project manager (replaces pip/poetry/pipenv).' },
  { slug: 'anthropic-cookbook', clone_url: 'https://github.com/anthropics/anthropic-cookbook', spdx: 'MIT', description: 'Anthropic Cookbook — official Claude API recipes.' },
  { slug: 'anthropic-sdk-python', clone_url: 'https://github.com/anthropics/anthropic-sdk-python', spdx: 'MIT', description: 'Official Python SDK for the Anthropic API.' },
  { slug: 'modelcontextprotocol', clone_url: 'https://github.com/modelcontextprotocol/specification', spdx: 'MIT', description: 'MCP specification — the protocol ministr speaks.' },
  { slug: 'deno', clone_url: 'https://github.com/denoland/deno', spdx: 'MIT', description: 'Deno — modern runtime for JS/TS, secure by default.' },
  { slug: 'bun', clone_url: 'https://github.com/oven-sh/bun', spdx: 'MIT', description: 'Bun — JS runtime, bundler, test runner, package manager.' },
  { slug: 'nodejs', clone_url: 'https://github.com/nodejs/node', spdx: 'MIT', description: 'Node.js — JavaScript runtime built on V8.' },
  { slug: 'tauri', clone_url: 'https://github.com/tauri-apps/tauri', spdx: 'MIT', description: 'Tauri — Rust desktop apps with web frontends.' },
];

export function GET() {
  const snapshot: ManifestSnapshot = {
    schema_version: 1,
    count: ATLAS.length,
    entries: ATLAS.map((e) => ({ ...e, opted_out: false })),
  };
  return NextResponse.json(snapshot, {
    headers: {
      // Long cache; the cron-driven server-side manifest is the live
      // view. This static mirror is a transparency artefact that
      // changes when the seed list changes (rare).
      'Cache-Control': 'public, max-age=3600, s-maxage=3600',
    },
  });
}

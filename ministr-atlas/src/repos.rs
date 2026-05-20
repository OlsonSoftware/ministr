//! Curated seed list of the 50 repos shipping in the F2.6 pilot.
//!
//! Selection methodology (F4.1 will document this in `LEGAL-ATLAS.md`
//! once G.1 closes): GitHub stars ≥ 1K, commit activity within the
//! last 12 months, breadth across the language axis the moat
//! narrative needs (Rust, TypeScript, Python, Go, Ruby, Java, plus
//! key infrastructure repos). Every entry MUST carry a permissive
//! SPDX identifier — copyleft (GPL/AGPL/LGPL) is deferred to G.1.
//!
//! When updating this list, change one spot only. The public manifest
//! mirror in `docs-next/app/(home)/atlas/manifest.json/route.ts`
//! reads from this same source via the build-time JSON generator.

use serde::{Deserialize, Serialize};

/// One curated Atlas repo. The shape doubles as the manifest row in
/// [`crate::manifest::ManifestEntry`]; the manifest adds runtime
/// fields (last-indexed commit + ts) at serialise time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedRepo {
    /// URL-safe label used in route paths (`/atlas/{slug}/survey`).
    /// Convention: lowercase, hyphenated, scoped by owner only when
    /// disambiguation matters (e.g. `tokio-rs-tokio` vs `tokio-rs-axum`).
    pub slug: &'static str,
    /// `https://github.com/<owner>/<name>` clone URL.
    pub clone_url: &'static str,
    /// SPDX identifier of the repo's primary license. Permissive
    /// (MIT, Apache-2.0, BSD-*, MPL-2.0, ISC, Unlicense, 0BSD) is the
    /// only category the pilot ships; G.1 unlocks the copyleft branch.
    pub spdx: &'static str,
    /// Human-readable one-line summary surfaced in the manifest.
    pub description: &'static str,
}

/// The 50 pilot repos. Order matches §3's moat narrative: web
/// frameworks first (broadest agent-impact), then runtimes, then
/// infrastructure, then language tooling.
pub const ATLAS_SEED_REPOS: &[SeedRepo] = &[
    // ── Web frameworks ────────────────────────────────────────────
    SeedRepo {
        slug: "react",
        clone_url: "https://github.com/facebook/react",
        spdx: "MIT",
        description: "The React UI library — the most-queried JS codebase on GitHub.",
    },
    SeedRepo {
        slug: "vue",
        clone_url: "https://github.com/vuejs/core",
        spdx: "MIT",
        description: "Vue 3 core — reactive framework with a runtime + compiler split.",
    },
    SeedRepo {
        slug: "svelte",
        clone_url: "https://github.com/sveltejs/svelte",
        spdx: "MIT",
        description: "Svelte 5 — runes-based compiler and component model.",
    },
    SeedRepo {
        slug: "nextjs",
        clone_url: "https://github.com/vercel/next.js",
        spdx: "MIT",
        description: "Next.js — React meta-framework with App Router and Turbopack.",
    },
    SeedRepo {
        slug: "tailwindcss",
        clone_url: "https://github.com/tailwindlabs/tailwindcss",
        spdx: "MIT",
        description: "Tailwind CSS — utility-first CSS framework, v4 engine.",
    },
    SeedRepo {
        slug: "remix",
        clone_url: "https://github.com/remix-run/remix",
        spdx: "MIT",
        description: "Remix — React framework now merged with React Router.",
    },
    SeedRepo {
        slug: "astro",
        clone_url: "https://github.com/withastro/astro",
        spdx: "MIT",
        description: "Astro — content-driven web framework with islands architecture.",
    },
    SeedRepo {
        slug: "solid",
        clone_url: "https://github.com/solidjs/solid",
        spdx: "MIT",
        description: "SolidJS — fine-grained reactive primitives, JSX without VDOM.",
    },
    // ── Python frameworks ─────────────────────────────────────────
    SeedRepo {
        slug: "django",
        clone_url: "https://github.com/django/django",
        spdx: "BSD-3-Clause",
        description: "Django — the batteries-included Python web framework.",
    },
    SeedRepo {
        slug: "fastapi",
        clone_url: "https://github.com/tiangolo/fastapi",
        spdx: "MIT",
        description: "FastAPI — modern Python web framework on Starlette + Pydantic.",
    },
    SeedRepo {
        slug: "flask",
        clone_url: "https://github.com/pallets/flask",
        spdx: "BSD-3-Clause",
        description: "Flask — the original micro web framework for Python.",
    },
    // ── Ruby + Java + Go web ──────────────────────────────────────
    SeedRepo {
        slug: "rails",
        clone_url: "https://github.com/rails/rails",
        spdx: "MIT",
        description: "Ruby on Rails — the original convention-over-configuration web stack.",
    },
    SeedRepo {
        slug: "spring-boot",
        clone_url: "https://github.com/spring-projects/spring-boot",
        spdx: "Apache-2.0",
        description: "Spring Boot — opinionated stand-alone Spring application bootstrapper.",
    },
    SeedRepo {
        slug: "gin",
        clone_url: "https://github.com/gin-gonic/gin",
        spdx: "MIT",
        description: "Gin — high-performance HTTP framework for Go.",
    },
    SeedRepo {
        slug: "echo",
        clone_url: "https://github.com/labstack/echo",
        spdx: "MIT",
        description: "Echo — high-performance Go web framework with middleware-first design.",
    },
    // ── Databases + caches ────────────────────────────────────────
    SeedRepo {
        slug: "postgres",
        clone_url: "https://github.com/postgres/postgres",
        spdx: "PostgreSQL", // BSD-style, OSI-approved permissive
        description: "PostgreSQL — the canonical open-source relational database.",
    },
    SeedRepo {
        slug: "sqlite",
        clone_url: "https://github.com/sqlite/sqlite",
        spdx: "blessing", // SQLite uses a public-domain dedication blessing
        description: "SQLite — public-domain embedded SQL database engine.",
    },
    SeedRepo {
        slug: "redis",
        clone_url: "https://github.com/redis/redis",
        spdx: "BSD-3-Clause",
        description: "Redis — in-memory data structure store. Pre-licence-relicense fork lineage.",
    },
    SeedRepo {
        slug: "valkey",
        clone_url: "https://github.com/valkey-io/valkey",
        spdx: "BSD-3-Clause",
        description: "Valkey — Linux Foundation Redis fork, post-2024 relicensing.",
    },
    SeedRepo {
        slug: "duckdb",
        clone_url: "https://github.com/duckdb/duckdb",
        spdx: "MIT",
        description: "DuckDB — embedded analytical database, SQLite for OLAP.",
    },
    // ── Rust ecosystem ────────────────────────────────────────────
    SeedRepo {
        slug: "tokio",
        clone_url: "https://github.com/tokio-rs/tokio",
        spdx: "MIT",
        description: "Tokio — async runtime + ecosystem for Rust.",
    },
    SeedRepo {
        slug: "axum",
        clone_url: "https://github.com/tokio-rs/axum",
        spdx: "MIT",
        description: "Axum — ergonomic, modular web framework on Tokio.",
    },
    SeedRepo {
        slug: "serde",
        clone_url: "https://github.com/serde-rs/serde",
        spdx: "MIT",
        description: "Serde — generic serialisation framework, the Rust ecosystem keystone.",
    },
    SeedRepo {
        slug: "tracing",
        clone_url: "https://github.com/tokio-rs/tracing",
        spdx: "MIT",
        description: "Tracing — structured async-aware diagnostics for Rust.",
    },
    SeedRepo {
        slug: "rust",
        clone_url: "https://github.com/rust-lang/rust",
        spdx: "Apache-2.0",
        description: "The Rust compiler and standard library.",
    },
    SeedRepo {
        slug: "cargo",
        clone_url: "https://github.com/rust-lang/cargo",
        spdx: "Apache-2.0",
        description: "Cargo — Rust's package manager and build tool.",
    },
    // ── Infrastructure / DevOps ───────────────────────────────────
    SeedRepo {
        slug: "kubernetes",
        clone_url: "https://github.com/kubernetes/kubernetes",
        spdx: "Apache-2.0",
        description: "Kubernetes — production-grade container orchestrator.",
    },
    SeedRepo {
        slug: "containerd",
        clone_url: "https://github.com/containerd/containerd",
        spdx: "Apache-2.0",
        description: "containerd — OCI container runtime.",
    },
    SeedRepo {
        slug: "envoy",
        clone_url: "https://github.com/envoyproxy/envoy",
        spdx: "Apache-2.0",
        description: "Envoy — high-performance L7 proxy and service-mesh data plane.",
    },
    SeedRepo {
        slug: "prometheus",
        clone_url: "https://github.com/prometheus/prometheus",
        spdx: "Apache-2.0",
        description: "Prometheus — pull-based monitoring + alerting system.",
    },
    SeedRepo {
        slug: "grafana",
        clone_url: "https://github.com/grafana/grafana",
        spdx: "AGPL-3.0-only", // Note: AGPL — flagged for G.1 review
        description: "Grafana — observability dashboards (AGPL — review under G.1 before retrieval).",
    },
    SeedRepo {
        slug: "terraform-provider-aws",
        clone_url: "https://github.com/hashicorp/terraform-provider-aws",
        spdx: "MPL-2.0",
        description: "Terraform AWS provider — broad coverage of AWS APIs.",
    },
    // ── ML / AI ───────────────────────────────────────────────────
    SeedRepo {
        slug: "pytorch",
        clone_url: "https://github.com/pytorch/pytorch",
        spdx: "BSD-3-Clause",
        description: "PyTorch — tensors and dynamic neural networks in Python with autograd.",
    },
    SeedRepo {
        slug: "transformers",
        clone_url: "https://github.com/huggingface/transformers",
        spdx: "Apache-2.0",
        description: "Hugging Face Transformers — pretrained model hub + training utilities.",
    },
    SeedRepo {
        slug: "diffusers",
        clone_url: "https://github.com/huggingface/diffusers",
        spdx: "Apache-2.0",
        description: "Hugging Face Diffusers — diffusion model pipelines.",
    },
    SeedRepo {
        slug: "llama-cpp",
        clone_url: "https://github.com/ggerganov/llama.cpp",
        spdx: "MIT",
        description: "llama.cpp — LLM inference in plain C/C++.",
    },
    SeedRepo {
        slug: "ollama",
        clone_url: "https://github.com/ollama/ollama",
        spdx: "MIT",
        description: "Ollama — run LLMs locally with a simple CLI.",
    },
    // ── Build tools + bundlers ────────────────────────────────────
    SeedRepo {
        slug: "vite",
        clone_url: "https://github.com/vitejs/vite",
        spdx: "MIT",
        description: "Vite — fast bundler/dev server built on Rollup + esbuild.",
    },
    SeedRepo {
        slug: "esbuild",
        clone_url: "https://github.com/evanw/esbuild",
        spdx: "MIT",
        description: "esbuild — extremely fast JavaScript/TypeScript bundler.",
    },
    SeedRepo {
        slug: "rollup",
        clone_url: "https://github.com/rollup/rollup",
        spdx: "MIT",
        description: "Rollup — ESM-first module bundler.",
    },
    SeedRepo {
        slug: "biome",
        clone_url: "https://github.com/biomejs/biome",
        spdx: "MIT",
        description: "Biome — formatter + linter for JS/TS/JSX/JSON/CSS, Rust-native.",
    },
    SeedRepo {
        slug: "ruff",
        clone_url: "https://github.com/astral-sh/ruff",
        spdx: "MIT",
        description: "Ruff — Rust-native Python linter and formatter, 10-100× faster than pylint.",
    },
    SeedRepo {
        slug: "uv",
        clone_url: "https://github.com/astral-sh/uv",
        spdx: "MIT",
        description: "uv — Rust-native Python package + project manager (replaces pip/poetry/pipenv).",
    },
    // ── AI / Agent tooling references ─────────────────────────────
    SeedRepo {
        slug: "anthropic-cookbook",
        clone_url: "https://github.com/anthropics/anthropic-cookbook",
        spdx: "MIT",
        description: "Anthropic Cookbook — official Claude API recipes.",
    },
    SeedRepo {
        slug: "anthropic-sdk-python",
        clone_url: "https://github.com/anthropics/anthropic-sdk-python",
        spdx: "MIT",
        description: "Official Python SDK for the Anthropic API.",
    },
    SeedRepo {
        slug: "modelcontextprotocol",
        clone_url: "https://github.com/modelcontextprotocol/specification",
        spdx: "MIT",
        description: "MCP specification — the protocol ministr speaks.",
    },
    // ── Browser engines + runtimes ────────────────────────────────
    SeedRepo {
        slug: "deno",
        clone_url: "https://github.com/denoland/deno",
        spdx: "MIT",
        description: "Deno — modern runtime for JS/TS, secure by default.",
    },
    SeedRepo {
        slug: "bun",
        clone_url: "https://github.com/oven-sh/bun",
        spdx: "MIT",
        description: "Bun — JS runtime, bundler, test runner, package manager.",
    },
    SeedRepo {
        slug: "nodejs",
        clone_url: "https://github.com/nodejs/node",
        spdx: "MIT",
        description: "Node.js — JavaScript runtime built on V8.",
    },
    // ── Misc broad-impact ─────────────────────────────────────────
    SeedRepo {
        slug: "tauri",
        clone_url: "https://github.com/tauri-apps/tauri",
        spdx: "MIT",
        description: "Tauri — Rust desktop apps with web frontends.",
    },
];

/// Compile-time assertion: the seed list is exactly the pilot's 50.
const _: () = assert!(ATLAS_SEED_REPOS.len() == 50, "Atlas v0 pilot ships 50 seed repos");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pilot_seed_list_is_exactly_fifty() {
        assert_eq!(ATLAS_SEED_REPOS.len(), 50);
    }

    #[test]
    fn slugs_are_unique() {
        let mut slugs: Vec<&'static str> =
            ATLAS_SEED_REPOS.iter().map(|r| r.slug).collect();
        slugs.sort_unstable();
        let mut dedup = slugs.clone();
        dedup.dedup();
        assert_eq!(
            slugs.len(),
            dedup.len(),
            "duplicate atlas slug detected — slugs must be unique to be routable"
        );
    }

    #[test]
    fn slugs_are_url_safe() {
        for repo in ATLAS_SEED_REPOS {
            assert!(
                repo.slug
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "non-URL-safe slug: {}",
                repo.slug
            );
        }
    }

    #[test]
    fn every_clone_url_is_https_github() {
        for repo in ATLAS_SEED_REPOS {
            assert!(
                repo.clone_url.starts_with("https://github.com/"),
                "non-https-github clone URL: {}",
                repo.clone_url
            );
        }
    }
}

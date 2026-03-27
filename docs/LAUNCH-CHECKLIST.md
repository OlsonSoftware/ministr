# iris Launch Checklist

Manual steps to ship iris as an open source project. Do these in order.

## Week 1: Repo Prep

- [ ] **Audit git history for secrets** — run `gitleaks detect` or `trufflehog` on the full history. Remove any API keys, tokens, or personal paths that leaked into commits. Consider a fresh `git filter-repo` if needed.
- [ ] **Choose org or personal repo** — `github.com/alrik/iris` vs creating an `iris-rs` org? Org looks more serious and allows collaborators later.
- [ ] **Add LICENSE** — create `LICENSE-MIT` and `LICENSE-APACHE` in repo root. Update `Cargo.toml` workspace `license` field to `"MIT OR Apache-2.0"`.
- [ ] **Write README.md** — this is the single most important thing. Structure:
  1. One-line pitch: "iris — an MCP server that traces code across language boundaries"
  2. The squid2 bridge trace map (the Rust↔TypeScript table from the stress test session)
  3. 30-second install: `brew install alrik/tap/iris` or `cargo install iris-cli`
  4. 60-second setup: show `.iris.toml` and `.mcp.json`
  5. Feature list with code examples (survey, symbols, definition, references, bridge)
  6. Architecture diagram (3-crate workspace)
  7. Link to docs
- [ ] **Add CONTRIBUTING.md** — dev setup (`cargo build --workspace`), testing (`cargo test --workspace`), lint (`just validate`), architecture overview pointing to `docs/src/architecture.md`
- [ ] **Create .github/ISSUE_TEMPLATE/** — bug report and feature request templates
- [ ] **Create .github/PULL_REQUEST_TEMPLATE.md**

## Week 1: CI/CD

- [ ] **GitHub Actions CI** (`.github/workflows/ci.yml`):
  - Trigger: push to main, PRs
  - Matrix: `ubuntu-latest`, `macos-latest` (arm64), `windows-latest`
  - Steps: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --workspace`
  - Cache: `~/.cargo/registry`, `target/`
- [ ] **GitHub Actions Release** (`.github/workflows/release.yml`):
  - Trigger: push tag `v*`
  - Build: cross-compile for macOS arm64/x86_64, Linux x86_64/aarch64, Windows x86_64
  - Use `cross` or `cargo-zigbuild` for cross-compilation
  - Upload binaries + checksums as GitHub release assets
  - Note: ONNX runtime needs special handling per platform — test each target
- [ ] **Homebrew tap** — create `homebrew-iris` repo with a formula that downloads the GitHub release binary. Test on a clean machine.

## Week 1: Distribution

- [ ] **Publish to crates.io** — ensure all three crates have correct metadata (`description`, `repository`, `readme`, `keywords`, `categories`). Publish in order: `iris-core` → `iris-mcp` → `iris-cli`.
- [ ] **GitHub Sponsors** — enable on your profile. Set up tiers:
  - $5/mo — Supporter (name in README)
  - $25/mo — Backer (priority issue responses)
  - $100/mo — Sponsor (logo in README, direct access)

## Week 2: Launch Content

- [ ] **Record demo** — use asciinema or screen recording:
  - 60s version: `iris_clone` pydantic-core → `iris_bridge --bridge_kind pyo3` → 21 cross-language links appear
  - 2min version: create `.iris.toml` → indexing → survey → symbols → definition → references → bridge trace
- [ ] **Landing page** — GitHub Pages or simple Astro/Next site:
  - Demo GIF above the fold
  - `brew install` command
  - Feature comparison table (iris vs LSP vs grep)
  - "iris cloud" waitlist email capture (simple Mailchimp/Buttondown form)
- [ ] **Write blog post** — "Building a cross-language code tracer in Rust" — the technical story of bridge detection

## Week 2: Launch Day

- [ ] **Hacker News** — "Show HN: iris — MCP server that traces code across language boundaries" — post at 9am ET Tuesday or Wednesday
- [ ] **Reddit** — r/rust (technical focus), r/programming (general), r/ClaudeAI (MCP focus)
- [ ] **Twitter/X** — thread with the trace map, tag @AnthropicAI @rustlang
- [ ] **MCP registries** — submit PR to `awesome-mcp-servers`, list on MCP Market, Smithery
- [ ] **Discord** — post in Anthropic discord #mcp-tools, Rust community discord

## Post-Launch

- [ ] **Monitor issues** — respond to every issue within 24h for the first month
- [ ] **Ship a v0.2 within 2 weeks** — address the top 3 user-reported issues to show momentum
- [ ] **Collect testimonials** — screenshot positive tweets/comments for the landing page

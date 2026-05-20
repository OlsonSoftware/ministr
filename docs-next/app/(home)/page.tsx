import Link from 'next/link';
import { InstallTabs } from '@/components/landing/install-tabs';
import { ToolList } from '@/components/landing/tool-list';
import { CopyButton } from '@/components/landing/copy-button';
import { INSTALL_COMMANDS } from '@/lib/install';

// macOS one-liner doubles as the Linux one; the full matrix lives in
// the Install section below and on /install.
const CLI = INSTALL_COMMANDS.find((c) => c.id === 'macos')!.command;

/**
 * Landing — a manuscript. One reading column, hairline rules,
 * numbered sections, static low-fidelity figures. No motion, no
 * playback, no glass. The page argues a single claim in order:
 * grep can't see structure; ministr indexes it; here's the shape
 * of the thing; install it.
 */
export default function HomePage() {
  return (
    <main
      data-ministr-landing
      className="ministr-landing ministr-ms relative isolate overflow-x-hidden"
    >
      <article className="py-20 sm:py-28">
        {/* ── Masthead ─────────────────────────────────────── */}
        <header className="ms-col">
          <p className="ms-folio">A code intelligence MCP server</p>
          <h1 className="ms-wordmark mt-5">
            ministr<span>.</span>
          </h1>
          <p className="ms-standfirst mt-6">
            Real codebase understanding for AI coding agents.
          </p>
          <p className="ms-p mt-5">
            Claude Code, Cursor, and Copilot explore code with{' '}
            <code>grep</code> and <code>read</code> — text matching that
            misses meaning and returns whole files. ministr replaces that
            with a local model of the codebase: AST-level semantic search,
            symbol navigation, real reference graphs, and cross-language
            bridge detection across 40+ languages.
          </p>
          <div className="mt-8 flex flex-wrap items-center gap-x-7 gap-y-3 text-[15px]">
            <Link href="/install" className="ms-link font-medium">
              Install ministr →
            </Link>
            <Link href="/docs/getting-started" className="ms-link">
              Getting started
            </Link>
            <Link href="/docs/architecture" className="ms-link">
              Architecture
            </Link>
          </div>
          <div className="ms-cli mt-6">
            <span className="ms-prompt">$</span>
            <span>{CLI}</span>
            <CopyButton value={CLI} label="Copy install command" size="sm" />
          </div>
        </header>

        <Rule className="my-16 sm:my-20" />

        {/* ── §1 The problem ───────────────────────────────── */}
        <Section folio="§ 1" title="Grep finds text. It cannot find meaning.">
          <p className="ms-p">
            Ask an agent to find one function and it globs the repo, greps
            for a name, then reads whole files to see the match in context.
            Every textual hit comes back — the definition, the call sites,
            the comments, the unrelated namesake in another module — and the
            agent pays for all of it, every turn.
          </p>
          <p className="ms-p">
            Text search has no notion of a <strong>symbol</strong>. It can't
            tell a definition from a mention, a caller from a comment, or
            that a Rust <code>#[pyfunction]</code> is precisely what some
            Python file calls across the language boundary. Those are the
            questions that actually move work forward, and they are exactly
            the ones a regex can't answer.
          </p>

          <Figure
            className="mt-10"
            caption={
              <>
                <b>Fig. 1</b> The same task — &ldquo;find and edit{' '}
                <code>validate_email</code>&rdquo; — asked two ways. Left:
                what <code>grep</code> + <code>read</code> return. Right:
                what ministr returns. The token math is a{' '}
                <Link href="/docs/benchmarks" className="ms-link">
                  reproducible benchmark
                </Link>
                .
              </>
            }
          >
            <div className="ms-compare [container-type:inline-size]">
              <div>
                <h4>grep + read</h4>
                <ul>
                  <li>120 candidate paths from a glob; one matters</li>
                  <li>47 grep lines, each prefixed with a full path</li>
                  <li>a 340-line file read to see a 22-line function</li>
                  <li>the same file read again next turn — no memory</li>
                </ul>
              </div>
              <div>
                <h4>ministr</h4>
                <ul>
                  <li>
                    <code>ministr_symbols</code> — the symbol, its kind,
                    signature, docs
                  </li>
                  <li>
                    <code>ministr_references</code> — its real callers and
                    implementors
                  </li>
                  <li>
                    <code>ministr_definition</code> — just the function body
                  </li>
                  <li>
                    <code>ministr_bridge</code> — every call site across a
                    language boundary
                  </li>
                </ul>
              </div>
            </div>
          </Figure>
        </Section>

        <Rule className="my-16 sm:my-20" />

        {/* ── §2 What it is ────────────────────────────────── */}
        <Section
          folio="§ 2"
          title="A model of the codebase, served over MCP."
        >
          <p className="ms-p">
            ministr parses a repository into an AST and keeps two query
            surfaces over it that compose. Both run entirely on your
            machine.
          </p>
          <div className="ms-list mt-8">
            <Item
              h="Structural"
              p={
                <>
                  <code>ministr_symbols</code>, <code>ministr_definition</code>,{' '}
                  <code>ministr_references</code>,{' '}
                  <code>ministr_bridge</code>. Backed by a symbol table, a
                  resolved reference graph, and a cross-language bridge
                  linker covering 13 binding kinds — Tauri, napi-rs, PyO3,
                  wasm-bindgen, gRPC, HTTP, FFI, and more.
                </>
              }
            />
            <Item
              h="Semantic"
              p={
                <>
                  <code>ministr_survey</code>, <code>ministr_read</code>,{' '}
                  <code>ministr_extract</code>. Dense embeddings fused with
                  sparse keyword matching, reranked — and every document
                  indexed at several resolutions, so the agent gets the
                  exact section, not a file dump.
                </>
              }
            />
          </div>
        </Section>

        <Rule className="my-16 sm:my-20" />

        {/* ── §3 How it indexes ────────────────────────────── */}
        <Section folio="§ 3" title="How a repository becomes that model.">
          <p className="ms-p">
            Indexing runs in three stages. The middle one is what separates
            ministr from search.
          </p>

          <Figure
            className="mt-10"
            caption={
              <>
                <b>Fig. 2</b> The ingestion pipeline. Unchanged files are
                skipped by hash on re-index; only the parse-and-extract
                stage understands code structure.
              </>
            }
          >
            <pre>{`  ┌─ discover ──────┐   ┌─ understand ────────────┐   ┌─ index ─────────┐
  │ walk the files  │   │ parse to a syntax tree  │   │ meaning + words  │
  │ skip unchanged  │ ▸ │ symbols · references    │ ▸ │ local, on disk   │
  │                 │   │ cross-language bridges  │   │                  │
  └─────────────────┘   └─────────────────────────┘   └──────────────────┘
                              the part grep can't do`}</pre>
          </Figure>

          <p className="ms-p mt-8">
            The result is one local index: the code-intelligence model —
            symbols, reference edges, bridge links — and the searchable
            text live together, so a structural query and a semantic query
            resolve against the same thing. No code is sent to an API; a
            small embedding model is the only first-run download.
          </p>
        </Section>

        <Rule className="my-16 sm:my-20" />

        {/* ── §4 The tool surface ──────────────────────────── */}
        <Section
          folio="§ 4"
          title="What your agent can ask for."
        >
          <p className="ms-p">
            Fifteen MCP tools, grouped by what they do. Every one links to
            its reference page.
          </p>
          <div className="mt-9">
            <ToolList />
          </div>
        </Section>

        <Rule className="my-16 sm:my-20" />

        {/* ── §5 Architecture ──────────────────────────────── */}
        <Section folio="§ 5" title="The shape of it.">
          <p className="ms-p">
            The agent speaks ordinary MCP. Behind that, a local engine
            holds the model and answers — and a background process builds
            it once and shares it, so a second client doesn&rsquo;t load a
            second copy into memory.
          </p>

          <Figure
            className="mt-10"
            caption={
              <>
                <b>Fig. 3</b> Top to bottom, conceptually. More on the{' '}
                <Link href="/docs/architecture" className="ms-link">
                  architecture page
                </Link>
                .
              </>
            }
          >
            <pre>{`  agent ───────── ministr_symbols · references · bridge · survey
    │              (MCP — stdio or HTTP)
  ministr ─────── code model    symbols · references · bridges
    │             retrieval     meaning + keyword, ranked
    │
  local index ── on your machine; nothing leaves it`}</pre>
          </Figure>
        </Section>

        <Rule className="my-16 sm:my-20" />

        {/* ── §6 Install ───────────────────────────────────── */}
        <Section folio="§ 6" title="One installer. Every platform.">
          <p className="ms-p">
            Download, double-click, done — macOS, Windows, Linux. The{' '}
            <code>ministr</code> CLI lands on your PATH automatically. Any
            MCP client; 100% local.
          </p>
          <div className="mt-9">
            <InstallTabs />
          </div>
        </Section>

        <Rule className="my-16 sm:my-20" />

        {/* ── Coda ─────────────────────────────────────────── */}
        <footer className="ms-col">
          <p className="ms-standfirst">Give your agent eyes for structure.</p>
          <p className="mt-5">
            <Link href="/install" className="ms-link font-medium text-[15px]">
              Install ministr →
            </Link>
          </p>
          <nav className="mt-10 flex flex-wrap gap-x-6 gap-y-2 text-[14px]">
            <Link href="/docs/getting-started" className="ms-link">
              Getting started
            </Link>
            <Link href="/docs/tools" className="ms-link">
              Tool reference
            </Link>
            <Link href="/docs/architecture" className="ms-link">
              Architecture
            </Link>
            <Link href="/docs/concepts" className="ms-link">
              Concepts
            </Link>
            <Link href="/pricing" className="ms-link">
              Pricing
            </Link>
            <Link href="/stewardship" className="ms-link">
              Stewardship
            </Link>
          </nav>
          <p className="ms-folio mt-10">
            Local · Rust · no API calls · <Link href="/stewardship" className="ms-link">MIT core, paid cloud</Link>
          </p>
        </footer>
      </article>
    </main>
  );
}

function Rule({ className = '' }: { className?: string }) {
  return (
    <div className={'ms-col ' + className}>
      <hr className="ms-rule-line" />
    </div>
  );
}

function Section({
  folio,
  title,
  children,
}: {
  folio: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="ms-col">
      <p className="ms-folio">{folio}</p>
      <h2 className="ms-h">{title}</h2>
      <div className="mt-5">{children}</div>
    </section>
  );
}

function Item({ h, p }: { h: string; p: React.ReactNode }) {
  return (
    <div className="ms-item">
      <h3>{h}</h3>
      <p>{p}</p>
    </div>
  );
}

function Figure({
  children,
  caption,
  className = '',
}: {
  children: React.ReactNode;
  caption: React.ReactNode;
  className?: string;
}) {
  return (
    <figure className={'ms-figure ' + className}>
      {children}
      <figcaption className="ms-figcap">{caption}</figcaption>
    </figure>
  );
}

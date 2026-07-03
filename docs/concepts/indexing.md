# Indexing

## The pipeline

A corpus (your project's path set) is walked, parsed, and stored at several
granularities:

1. **Documents** — every supported file. Markdown, HTML, and PDF are parsed
   as prose; code files are parsed per language; unsupported files (images,
   binaries) are skipped. The walker respects `.gitignore` plus built-in
   ignore lists (build directories, lockfiles, bundles, model binaries), and
   `[corpus] ignore` globs add project-specific patterns (see
   [configuration](../guides/configuration.md)).
2. **Sections** — the retrieval unit. Prose sections follow headings; code
   files get one section per top-level symbol, split at inner AST boundaries
   when a symbol is too large for one section. Section IDs are hierarchical
   and human-readable: `src/auth.rs#auth::validate_token`,
   `docs/auth.md#error-handling`.
3. **Claims** — atomic factual assertions extracted per section. They power
   `ministr_extract` (cheaper than full text) and `ministr_related`
   (cross-reference edges).
4. **Symbols** — the code index: functions, structs, traits, enums, impls,
   modules, and the rest, each with visibility, signature, doc comment, and
   line range. References are tracked by kind: calls, implements, imports,
   uses, bridge.

With `sparse_weight > 0`, ingestion also builds the sparse keyword index
used by [hybrid search](search.md).

## Languages

Symbol extraction uses tree-sitter grammars across 40+ languages. File
types without a grammar still index at text level — searchable, but with no
symbol graph.

## Staying current

- **Incremental** — unchanged files are skipped by content hash, and a
  whole-tree check short-circuits no-op reindexes entirely.
- **Watching** — corpora are watched for changes; edits reindex through a
  coherence engine that also notifies connected agents (`coherence_alerts`
  in tool responses) when content they already received has changed.
- **Self-healing** — extractor version stamps force a re-parse after ministr
  upgrades change extraction behavior, even for unchanged files.

What "current" means to you is the [freshness](freshness.md) contract.

## Storage

One SQLite database per corpus under `~/.ministr` holds documents, sections,
claims, symbols, references, bridge links, file hashes, and sessions. The
HNSW vector index is a derived cache, rebuilt from the store whenever a
persisted snapshot can't be proven consistent.

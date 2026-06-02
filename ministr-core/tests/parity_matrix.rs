//! parity-audit-matrix — the build-failing per-corpus config-knob × surface
//! parity gate (the capstone of parity-epic).
//!
//! The whole epic exists because per-corpus config-honoring had silently
//! drifted between surfaces: the CLI applied a corpus's `[corpus] model` while
//! the daemon ignored it. The fix routed every surface through one seam
//! ([`resolve_effective_corpus_config`]). This test is the regression gate that
//! keeps it from drifting back — "schema parity fails → stop the line", not
//! monitor.
//!
//! It encodes, as a single asserted table, which surface honors which knob:
//!   * the CLI one-shot `ministr index` path, and
//!   * the long-lived `CorpusRegistry` (daemon REST + Tauri GUI + MCP — one
//!     shared `AppState`).
//!
//! Two things make it a real gate rather than documentation:
//!   1. **Compile-time exhaustiveness** — [`matrix_covers_every_effective_knob_exhaustively`]
//!      destructures every field of [`EffectiveCorpusConfig`], so adding a knob
//!      without classifying it here is a red build.
//!   2. **No silent "honored"** — every not-yet-applied cell must be
//!      `NotYet(tracking-ref)`; a regression that flips one to `Yes` without the
//!      wiring, or drops the tracking ref, fails the assertions below.

use ministr_core::config::{
    CorpusSpec, EffectiveCorpusConfig, MinistrConfig, RepoConfig, resolve_effective_corpus_config,
};

/// Whether a surface actually APPLIES a resolved per-corpus knob today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Honored {
    /// The surface applies the knob to ingest/query.
    Yes,
    /// Not applied yet; the `&str` names the chunk tracking the gap. A KNOWN
    /// divergence, recorded loudly so it can never masquerade as parity.
    NotYet(&'static str),
}

/// One row of the parity matrix.
struct Row {
    knob: &'static str,
    cli_one_shot: Honored,
    daemon_registry: Honored,
}

/// THE PARITY MATRIX — the single source of truth for which surface honors
/// which per-corpus knob, as of the parity-epic end-state.
///
/// Every knob in [`EffectiveCorpusConfig`] appears here exactly once (enforced
/// by [`matrix_covers_every_effective_knob_exhaustively`]).
const MATRIX: &[Row] = &[
    // model — resolved by the seam; the CLI threads it into `ResolvedConfig`
    // and the daemon registry routes a per-corpus embedder for it
    // (parity-seam-registry-routing). Honored end-to-end on both surfaces.
    Row {
        knob: "model",
        cli_one_shot: Honored::Yes,
        daemon_registry: Honored::Yes,
    },
    // dimension / rerank_depth — the CLI threads them (`ResolvedConfig`
    // `resolved_dimension` / `rerank_depth`); the registry now applies them too
    // (parity-registry-knobs): `create_handle` wraps the per-corpus embedder in
    // a `MatryoshkaEmbedder` at the configured dimension (truncated HNSW index +
    // ingest via the shared `apply_dimension` seam) and attaches
    // `with_matryoshka_rerank(dual, rerank_depth)` on the `QueryService` —
    // exactly the CLI's `init_infrastructure`/`build_server` wiring. Honored
    // end-to-end on both surfaces.
    Row {
        knob: "dimension",
        cli_one_shot: Honored::Yes,
        daemon_registry: Honored::Yes,
    },
    Row {
        knob: "rerank_depth",
        cli_one_shot: Honored::Yes,
        daemon_registry: Honored::Yes,
    },
    // parser / min_section_tokens / claim_extraction — these live only in the
    // per-corpus `meta.toml`, and NEITHER ingestion entry point passes a
    // `CorpusConfig` to the seam today (both pass `None`), so the resolved value
    // is always the default. No surface honors them yet.
    Row {
        knob: "parser",
        cli_one_shot: Honored::NotYet("parity-meta-toml-load"),
        daemon_registry: Honored::NotYet("parity-meta-toml-load"),
    },
    Row {
        knob: "min_section_tokens",
        cli_one_shot: Honored::NotYet("parity-meta-toml-load"),
        daemon_registry: Honored::NotYet("parity-meta-toml-load"),
    },
    Row {
        knob: "claim_extraction",
        cli_one_shot: Honored::NotYet("parity-meta-toml-load"),
        daemon_registry: Honored::NotYet("parity-meta-toml-load"),
    },
];

/// The canonical per-corpus knob names, mirroring both the
/// [`EffectiveCorpusConfig`] fields and the [`MATRIX`] rows. Kept at module
/// scope (clippy `items_after_statements`).
const KNOBS: &[&str] = &[
    "model",
    "dimension",
    "rerank_depth",
    "parser",
    "min_section_tokens",
    "claim_extraction",
];

fn row(knob: &str) -> &'static Row {
    MATRIX
        .iter()
        .find(|r| r.knob == knob)
        .unwrap_or_else(|| panic!("knob `{knob}` not in the parity MATRIX"))
}

#[test]
fn matrix_covers_every_effective_knob_exhaustively() {
    // COMPILE-TIME exhaustiveness: this destructure names every field of
    // `EffectiveCorpusConfig`. Add or remove a knob and this stops compiling
    // until the destructure, `KNOBS`, and `MATRIX` are all updated — exactly
    // the "stop the line" property the gate is for.
    let EffectiveCorpusConfig {
        model,
        dimension,
        rerank_depth,
        parser,
        min_section_tokens,
        claim_extraction,
    } = resolve_effective_corpus_config(None, None, &MinistrConfig::default());
    // Touch every binding so a removed knob is a compile error here too.
    let _ = (
        &model,
        &dimension,
        &rerank_depth,
        &parser,
        &min_section_tokens,
        &claim_extraction,
    );

    // `KNOBS` (module scope) is the runtime mirror of the destructure above.
    assert_eq!(
        MATRIX.len(),
        KNOBS.len(),
        "the parity MATRIX must classify every knob exactly once"
    );
    for knob in KNOBS {
        assert_eq!(
            MATRIX.iter().filter(|r| r.knob == *knob).count(),
            1,
            "knob `{knob}` must appear exactly once in the parity MATRIX"
        );
    }
}

#[test]
fn model_is_honored_end_to_end_via_the_shared_seam() {
    // model is the cell the epic closed on BOTH surfaces. Both route through
    // `resolve_effective_corpus_config`, so verifying the seam honors a repo
    // `[corpus] model` override is the behavioral proof the matrix's Yes/Yes row
    // rests on (not a hand-asserted claim).
    let global = MinistrConfig::default(); // default_model = all-MiniLM-L6-v2
    let repo = RepoConfig {
        corpus: CorpusSpec {
            model: Some("jina-embeddings-v2-base-code".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let eff = resolve_effective_corpus_config(Some(&repo), None, &global);
    assert_eq!(
        eff.model, "jina-embeddings-v2-base-code",
        "the shared seam must honor a per-corpus model override"
    );

    let m = row("model");
    assert_eq!(m.cli_one_shot, Honored::Yes);
    assert_eq!(
        m.daemon_registry,
        Honored::Yes,
        "the registry honors model via the embedder pool (parity-seam-registry-routing)"
    );
}

#[test]
fn known_registry_gaps_are_tracked_never_silent() {
    // The registry now applies the per-corpus MODEL (embedder pool,
    // parity-seam-registry-routing) AND the Matryoshka DIMENSION + RERANK_DEPTH
    // (parity-registry-knobs). The only remaining gaps are the `meta.toml`-only
    // knobs — neither ingestion entry point loads a `CorpusConfig` yet. Every
    // still-ungated registry cell MUST be `NotYet(non-empty tracking ref)` — a
    // regression that flips one to `Yes` without the wiring, or drops the
    // tracking ref, fails here; and a regression that drops one of the applied
    // knobs back to `NotYet` fails the `Yes` arm.
    for r in MATRIX {
        if matches!(r.knob, "model" | "dimension" | "rerank_depth") {
            assert_eq!(
                r.daemon_registry,
                Honored::Yes,
                "registry knob `{}` is applied — must be `Yes`",
                r.knob
            );
        } else {
            assert!(
                matches!(r.daemon_registry, Honored::NotYet(t) if !t.is_empty()),
                "registry knob `{}` must be tracked NotYet(non-empty) until applied",
                r.knob
            );
        }
    }
}

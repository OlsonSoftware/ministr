// The homepage "Four things grep can't do" section, rebuilt as a bespoke
// capability grid. Each capability gets its own signature micro-diagram in the
// v2 single-amber language (hairline, mono, sharp corners, tone on non-text
// marks only) so the section reads as a figure rather than four flat cards.

/* ── Glyph 1 · Structural — a symbol node with callers in, callees out ───── */
function StructuralGlyph() {
  return (
    <svg
      viewBox="0 0 148 56"
      role="img"
      aria-label="A central symbol node with caller edges flowing in from the left and callee edges flowing out to the right."
      className="v2-cap-svg"
    >
      <defs>
        <marker
          id="v2-cap-arrow"
          viewBox="0 0 10 10"
          refX="8.5"
          refY="5"
          markerWidth="6.5"
          markerHeight="6.5"
          orient="auto-start-reverse"
        >
          <path d="M0,0 L10,5 L0,10 z" className="v2-cap-arrowhead" />
        </marker>
      </defs>
      {/* caller edges (point into the centre) */}
      <line x1="20" y1="15" x2="66" y2="26" className="v2-cap-edge" markerEnd="url(#v2-cap-arrow)" />
      <line x1="20" y1="41" x2="66" y2="30" className="v2-cap-edge" markerEnd="url(#v2-cap-arrow)" />
      {/* callee edges (point out of the centre) */}
      <line x1="82" y1="26" x2="128" y2="15" className="v2-cap-edge" markerEnd="url(#v2-cap-arrow)" />
      <line x1="82" y1="30" x2="128" y2="41" className="v2-cap-edge" markerEnd="url(#v2-cap-arrow)" />
      {/* caller + callee nodes */}
      <circle cx="16" cy="15" r="4" className="v2-cap-node-dim" />
      <circle cx="16" cy="41" r="4" className="v2-cap-node-dim" />
      <circle cx="132" cy="15" r="4" className="v2-cap-node-dim" />
      <circle cx="132" cy="41" r="4" className="v2-cap-node-dim" />
      {/* the symbol itself */}
      <circle cx="74" cy="28" r="6.5" className="v2-cap-node-core" />
    </svg>
  );
}

/* ── Glyph 2 · Semantic — one lit section among a dim file dump ──────────── */
function SemanticGlyph() {
  return (
    <svg
      viewBox="0 0 148 56"
      role="img"
      aria-label="A stack of dim file lines with a single highlighted section box — the matched section, not the whole file."
      className="v2-cap-svg"
    >
      {/* the file dump: dim lines of varying length */}
      <line x1="6" y1="8" x2="126" y2="8" className="v2-cap-line-dim" />
      <line x1="6" y1="17" x2="98" y2="17" className="v2-cap-line-dim" />
      <line x1="6" y1="39" x2="118" y2="39" className="v2-cap-line-dim" />
      <line x1="6" y1="48" x2="84" y2="48" className="v2-cap-line-dim" />
      {/* the section that matters */}
      <rect x="6" y="23" width="126" height="11" className="v2-cap-sect" />
      <rect x="6" y="23" width="3" height="11" className="v2-cap-sect-tick" />
      <line x1="16" y1="28.5" x2="70" y2="28.5" className="v2-cap-line-lit" />
      <line x1="76" y1="28.5" x2="104" y2="28.5" className="v2-cap-line-lit" />
    </svg>
  );
}

/* ── Glyph 3 · Cross-language — a call hopping RS → PY → TS ──────────────── */
function CrossLangGlyph() {
  const tags = [
    { x: 6, label: "RS" },
    { x: 55, label: "PY" },
    { x: 104, label: "TS" },
  ];
  return (
    <svg
      viewBox="0 0 148 56"
      role="img"
      aria-label="A call hopping across three languages: a Rust box bridges into a Python box, which bridges into a TypeScript box."
      className="v2-cap-svg"
    >
      <defs>
        <marker
          id="v2-cap-arrow2"
          viewBox="0 0 10 10"
          refX="8.5"
          refY="5"
          markerWidth="6.5"
          markerHeight="6.5"
          orient="auto-start-reverse"
        >
          <path d="M0,0 L10,5 L0,10 z" className="v2-cap-arrowhead" />
        </marker>
      </defs>
      <line x1="44" y1="28" x2="55" y2="28" className="v2-cap-edge" markerEnd="url(#v2-cap-arrow2)" />
      <line x1="93" y1="28" x2="104" y2="28" className="v2-cap-edge" markerEnd="url(#v2-cap-arrow2)" />
      {tags.map((t) => (
        <g key={t.label}>
          <rect x={t.x} y="17" width="38" height="22" className="v2-cap-node-dim" />
          <text x={t.x + 19} y="32" textAnchor="middle" className="v2-cap-tag">
            {t.label}
          </text>
        </g>
      ))}
    </svg>
  );
}

/* ── Glyph 4 · Instant — a fast response spark with a sub-ms readout ─────── */
function InstantGlyph() {
  return (
    <svg
      viewBox="0 0 148 56"
      role="img"
      aria-label="A rapid response spark resolving to a sub-millisecond latency readout."
      className="v2-cap-svg"
    >
      <polyline
        points="6,44 18,44 26,18 34,44 48,44 56,30 64,44 76,44 84,14 94,44"
        className="v2-cap-spark"
      />
      <text x="104" y="33" className="v2-cap-readout">
        {"<1ms"}
      </text>
    </svg>
  );
}

interface Cap {
  index: string;
  title: string;
  body: string;
  foil: string;
  Glyph: () => React.JSX.Element;
}

const CAPS: Cap[] = [
  {
    index: "01",
    title: "Structural",
    body: "Symbols, definitions, callers. The questions grep can’t answer.",
    foil: "grep returns lines, not callers.",
    Glyph: StructuralGlyph,
  },
  {
    index: "02",
    title: "Semantic",
    body:
      "Search by meaning, not text matching. Your agent gets the section that matters, not a 300-line file dump.",
    foil: "grep dumps the whole file.",
    Glyph: SemanticGlyph,
  },
  {
    index: "03",
    title: "Cross-language",
    body:
      "Follow calls across Rust, Python, TypeScript, and dozens more, through 13 bridge kinds including PyO3, Tauri, napi-rs, FFI, and gRPC.",
    foil: "grep stops at the file boundary.",
    Glyph: CrossLangGlyph,
  },
  {
    index: "04",
    title: "Instant",
    body:
      "Bare-metal local indexing means queries return in milliseconds. Files auto-reindex on change, so results are never stale.",
    foil: "grep rescans from scratch every time.",
    Glyph: InstantGlyph,
  },
];

export function FourThingsGrid() {
  return (
    <div className="v2-caps">
      {CAPS.map(({ index, title, body, foil, Glyph }) => (
        <div className="v2-cap" key={index}>
          <div className="v2-cap-glyph">
            <Glyph />
          </div>
          <div className="v2-cap-head">
            <span className="v2-cap-index">{index}</span>
            <h3>{title}</h3>
          </div>
          <p>{body}</p>
          <p className="v2-cap-foil">{foil}</p>
        </div>
      ))}
    </div>
  );
}

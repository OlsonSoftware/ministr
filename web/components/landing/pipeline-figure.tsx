// The homepage architecture figure: a single 7-stage pipeline that shows how a
// repo becomes a millisecond answer. The amber-filled `index` node is the
// pivot — everything left of it happens ONCE per file change, everything right
// of it happens on EVERY query. Built in the v2 single-amber language (hairline,
// mono, sharp corners, tone on non-text marks only) and distinct from the
// branching cross-language bridge: this is a linear pipeline with two zone
// eyebrows and a hinge. Stage names are grounded in the real IngestionPipeline
// (parse -> embed -> HNSW insert) and QueryService (retrieve -> rerank -> cite).

interface Stage {
  name: string;
  sub: string;
  pivot?: boolean;
}

const STAGES: Stage[] = [
  { name: "repo", sub: "your code" },
  { name: "parse", sub: "tree-sitter" },
  { name: "embed", sub: "local model" },
  { name: "index", sub: "HNSW + graph", pivot: true },
  { name: "retrieve", sub: "cosine top-k" },
  { name: "rank", sub: "rerank" },
  { name: "answer", sub: "cited section" },
];

const W = 98; // node width
const H = 46; // node height
const PITCH = 140;
const X0 = 8;
const NODE_Y = 104;
const MID_Y = NODE_Y + H / 2; // 127

const nodeX = (i: number) => X0 + i * PITCH;
const center = (i: number) => nodeX(i) + W / 2;

const PIVOT_I = STAGES.findIndex((s) => s.pivot); // 3
const DIVIDER_X = center(PIVOT_I); // 477

export function PipelineFigure() {
  return (
    <figure className="v2-pipe">
      <svg
        viewBox="0 0 956 196"
        role="img"
        aria-label="ministr's pipeline. Once per file change: your repo is parsed with tree-sitter, embedded by a local model, and stored in an HNSW vector and reference-graph index. Then on every query: it retrieves by cosine similarity, reranks the top-k, and returns the cited section in milliseconds."
        className="v2-pipe-svg"
      >
        <defs>
          <marker
            id="v2-pipe-arrow"
            viewBox="0 0 10 10"
            refX="8.5"
            refY="5"
            markerWidth="7"
            markerHeight="7"
            orient="auto-start-reverse"
          >
            <path d="M0,0 L10,5 L0,10 z" className="v2-pipe-arrowhead" />
          </marker>
        </defs>

        {/* Zone divider through the pivot — write side / read side. */}
        <line
          x1={DIVIDER_X}
          y1="92"
          x2={DIVIDER_X}
          y2="158"
          className="v2-pipe-divider"
        />

        {/* Zone eyebrows. */}
        <text x={center(1)} y="84" textAnchor="middle" className="v2-pipe-eyebrow">
          INDEXED ONCE · ON FILE CHANGE
        </text>
        <text x={center(5)} y="84" textAnchor="middle" className="v2-pipe-eyebrow">
          ANSWERED IN MS · EVERY QUERY
        </text>

        {/* Edges (drawn first so the node boxes sit on top of the line ends). */}
        {STAGES.slice(0, -1).map((_, i) => (
          <line
            key={`e${i}`}
            x1={nodeX(i) + W}
            y1={MID_Y}
            x2={nodeX(i + 1)}
            y2={MID_Y}
            className="v2-pipe-edge"
            markerEnd="url(#v2-pipe-arrow)"
          />
        ))}

        {/* Nodes + sublabels. */}
        {STAGES.map((s, i) => (
          <g key={s.name}>
            <rect
              x={nodeX(i)}
              y={NODE_Y}
              width={W}
              height={H}
              className={s.pivot ? "v2-pipe-node-pivot" : "v2-pipe-node"}
            />
            <text
              x={center(i)}
              y={MID_Y + 4.5}
              textAnchor="middle"
              className={s.pivot ? "v2-pipe-name-pivot" : "v2-pipe-name"}
            >
              {s.name}
            </text>
            <text x={center(i)} y="166" textAnchor="middle" className="v2-pipe-sub">
              {s.sub}
            </text>
          </g>
        ))}
      </svg>

      <figcaption className="v2-pipe-cap">
        Indexed once, on every file change — then reused for the life of your
        session. Each query is just retrieve, rank, cite, which is why answers
        come back in milliseconds.{" "}
        <span className="v2-pipe-foil">grep starts over on every search.</span>
      </figcaption>
    </figure>
  );
}

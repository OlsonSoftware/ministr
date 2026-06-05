/**
 * CrossLanguageBridge — ministr's signature capability, shown.
 *
 * The page names "follow calls across Rust, Python, TypeScript … 13 bridge
 * kinds" but never draws it. This is one call traced through the FFI boundary:
 * a TypeScript invoke() reaches a Rust Tauri command, which itself bridges into
 * Python (PyO3) and back into TypeScript (napi-rs). grep sees three unrelated
 * files; ministr sees one call graph.
 *
 * Built fresh in the v2 language — a SINGLE amber accent (edges, bridge-kind
 * labels, language tags), symbol names in ink, hairline node boxes, mono, sharp
 * corners. Deliberately NOT the old rainbow BridgeGraphHero. Static SVG (scales
 * by viewBox), zero client JS, reduced-motion-safe.
 */

interface Node {
  id: string;
  tag: 'TS' | 'RS' | 'PY';
  sym: string;
  x: number;
  y: number;
}

const W = 200;
const H = 54;

const NODES: Node[] = [
  { id: 'ts1', tag: 'TS', sym: "invoke('clone_repo')", x: 8, y: 84 },
  { id: 'rs', tag: 'RS', sym: 'clone_repo', x: 348, y: 84 },
  { id: 'py', tag: 'PY', sym: 'on_clone', x: 752, y: 16 },
  { id: 'ts2', tag: 'TS', sym: 'index_corpus', x: 752, y: 152 },
];

interface Edge {
  from: string;
  to: string;
  kind: string;
}

const EDGES: Edge[] = [
  { from: 'ts1', to: 'rs', kind: 'tauri_command' },
  { from: 'rs', to: 'py', kind: 'pyo3' },
  { from: 'rs', to: 'ts2', kind: 'napi' },
];

const byId = new Map(NODES.map((n) => [n.id, n]));
const cy = (n: Node) => n.y + H / 2;

export function CrossLanguageBridge() {
  return (
    <figure className="v2-bridge">
      <svg
        viewBox="0 0 960 222"
        role="img"
        aria-label="One call across three languages: a TypeScript invoke('clone_repo') reaches a Rust Tauri command clone_repo, which bridges via PyO3 into the Python on_clone and via napi-rs into the TypeScript index_corpus."
        className="v2-bridge-svg"
      >
        <defs>
          <marker
            id="v2-bridge-arrow"
            viewBox="0 0 10 10"
            refX="8.5"
            refY="5"
            markerWidth="7"
            markerHeight="7"
            orient="auto-start-reverse"
          >
            <path d="M0,0 L10,5 L0,10 z" className="v2-bridge-arrowhead" />
          </marker>
        </defs>

        {/* Edges first, so the node boxes sit on top of the line ends. */}
        {EDGES.map((e) => {
          const a = byId.get(e.from)!;
          const b = byId.get(e.to)!;
          const x1 = a.x + W;
          const y1 = cy(a);
          const x2 = b.x;
          const y2 = cy(b);
          const mx = (x1 + x2) / 2;
          const my = (y1 + y2) / 2;
          const labelW = e.kind.length * 7.2 + 12;
          return (
            <g key={`${e.from}-${e.to}`}>
              <line
                x1={x1}
                y1={y1}
                x2={x2}
                y2={y2}
                className="v2-bridge-edge"
                markerEnd="url(#v2-bridge-arrow)"
              />
              <rect
                x={mx - labelW / 2}
                y={my - 11}
                width={labelW}
                height={18}
                className="v2-bridge-kindbg"
              />
              <text x={mx} y={my + 2} textAnchor="middle" className="v2-bridge-kind">
                {e.kind}
              </text>
            </g>
          );
        })}

        {/* Nodes. */}
        {NODES.map((n) => (
          <g key={n.id}>
            <rect
              x={n.x}
              y={n.y}
              width={W}
              height={H}
              className="v2-bridge-node"
            />
            <text x={n.x + 14} y={n.y + 21} className="v2-bridge-tag">
              {n.tag}
            </text>
            <text x={n.x + 14} y={n.y + 39} className="v2-bridge-sym">
              {n.sym}
            </text>
          </g>
        ))}
      </svg>

      <figcaption className="v2-bridge-cap">
        One call, three languages — ministr follows it through the FFI boundary.
        13 bridge kinds: PyO3, Tauri, napi-rs, wasm-bindgen, gRPC, and more.{' '}
        <span className="v2-bridge-foil">grep stops at the file.</span>
      </figcaption>
    </figure>
  );
}

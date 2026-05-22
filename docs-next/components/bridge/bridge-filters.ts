// F3.6-c-i — pure filter helper for the bridge-graph visualizer.
//
// Three orthogonal axes:
// 1. **Language** — admit nodes whose `lang` is in the allow-set.
//    Cascades: edges touching a hidden node are also hidden.
// 2. **Bridge kind** — admit edges whose `kind` is in the allow-set.
//    Cascades: nodes that become orphans (no remaining edges) are
//    hidden so the canvas doesn't render dangling endpoints.
// 3. **File substring** — admit nodes whose `file` contains the
//    case-insensitive substring. Cascades the same as the language
//    axis.
//
// Empty allow-set for an axis = "no filter on that axis" (admit
// all). An empty file substring = admit all (the alternative —
// hiding everything when the input is empty — is a worse default
// for the demo UX).
//
// The helper is intentionally a pure function with no React imports
// so it remains testable without spinning up a test runner.

import type { LiveBridgeEdge, LiveBridgeNode } from '../landing/bridge-graph';

export interface BridgeFilters {
  /** When non-empty, only admit nodes whose `lang` is in this set. */
  languages: ReadonlySet<string>;
  /** When non-empty, only admit edges whose `kind` is in this set. */
  kinds: ReadonlySet<string>;
  /** Case-insensitive substring match against the node's `file`. */
  fileSubstring: string;
}

export interface BridgeData {
  nodes: ReadonlyArray<LiveBridgeNode>;
  edges: ReadonlyArray<LiveBridgeEdge>;
}

/** Build a filter set with all axes open ("admit all"). */
export function noFilters(): BridgeFilters {
  return { languages: new Set(), kinds: new Set(), fileSubstring: '' };
}

/**
 * Apply the filters to the data. The output is a fresh
 * `BridgeData` — never the same reference as the input even when
 * no filtering occurs (callers can rely on referential equality to
 * detect change).
 *
 * Algorithm:
 *   1. Filter nodes by language + file substring → `survivingNodes`.
 *   2. Filter edges by kind → `kindFilteredEdges`.
 *   3. Drop edges whose `from`/`to` aren't in `survivingNodes`
 *      → `finalEdges` (cascade from language / file).
 *   4. Drop nodes that don't appear in any `finalEdges` AND were not
 *      explicitly admitted (i.e., still part of `survivingNodes`).
 *      v0 keeps a node even when it becomes orphaned by the kind
 *      filter ONLY if it survived all OTHER filters — this means a
 *      node admitted by lang + file but with no surviving edges
 *      still renders (helpful when the user wants to see all nodes
 *      of a particular language even if their bridges are filtered
 *      out). Toggle the dropOrphans flag to flip this behaviour.
 */
export function applyBridgeFilters(
  data: BridgeData,
  filters: BridgeFilters,
  options: { dropOrphans?: boolean } = {},
): BridgeData {
  const { languages, kinds, fileSubstring } = filters;
  const dropOrphans = options.dropOrphans ?? false;

  const trimmedFile = fileSubstring.trim().toLowerCase();
  const langActive = languages.size > 0;
  const kindActive = kinds.size > 0;
  const fileActive = trimmedFile.length > 0;

  // Pass 1: per-axis node admission.
  const survivingNodes = data.nodes.filter((node) => {
    if (langActive && !languages.has(node.lang)) return false;
    if (fileActive) {
      const file = (node.file ?? '').toLowerCase();
      if (!file.includes(trimmedFile)) return false;
    }
    return true;
  });

  const survivingIds = new Set(survivingNodes.map((n) => n.id));

  // Pass 2: edges → kind filter + cascade-from-nodes.
  const finalEdges = data.edges.filter((edge) => {
    if (kindActive && !kinds.has(edge.kind)) return false;
    if (!survivingIds.has(edge.from)) return false;
    if (!survivingIds.has(edge.to)) return false;
    return true;
  });

  if (!dropOrphans) {
    return { nodes: survivingNodes, edges: finalEdges };
  }

  // Pass 3 (optional): drop nodes that have no surviving edges.
  const referenced = new Set<string>();
  for (const e of finalEdges) {
    referenced.add(e.from);
    referenced.add(e.to);
  }
  const finalNodes = survivingNodes.filter((n) => referenced.has(n.id));
  return { nodes: finalNodes, edges: finalEdges };
}

/** Derive the set of distinct language slugs present in the data —
 *  drives the language filter chip set. Sort alphabetically for a
 *  stable UI ordering. */
export function distinctLanguages(data: BridgeData): string[] {
  return Array.from(new Set(data.nodes.map((n) => n.lang))).sort();
}

/** Derive the set of distinct bridge-kind slugs present in the data
 *  — drives the kind filter chip set. */
export function distinctKinds(data: BridgeData): string[] {
  return Array.from(new Set(data.edges.map((e) => e.kind))).sort();
}

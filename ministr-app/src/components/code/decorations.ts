/**
 * Pure mapping from the symbol index to Shiki decorations.
 *
 * v1 precision boundary (Sourcegraph's precise-nav model): the only clickable
 * hot-zones are the *name occurrences of definitions the index knows*. For
 * each {@link SymbolSpan} we locate its name on its declaration line and wrap
 * exactly that range, tagging it with the symbol id so the viewer can resolve
 * a click without re-parsing. Unresolved identifiers are handled by the ⌘K
 * palette's `search_symbols` fallback, not here.
 *
 * Shiki throws on overlapping decorations, so ranges are sorted and any that
 * overlap an earlier one on the same line are dropped.
 */
import type { DecorationItem } from "shiki";
import type { SymbolSpan } from "../../lib/types";

interface Candidate {
  line: number;
  start: number;
  end: number;
  span: SymbolSpan;
}

export function buildSymbolDecorations(
  content: string,
  spans: SymbolSpan[],
): DecorationItem[] {
  const lines = content.split("\n");
  const candidates: Candidate[] = [];

  for (const span of spans) {
    const lineIdx = span.line_start - 1;
    if (lineIdx < 0 || lineIdx >= lines.length) continue;
    const text = lines[lineIdx];
    const col = text.indexOf(span.name);
    if (col < 0) continue; // name not on its declaration line — no hot-zone
    candidates.push({ line: lineIdx, start: col, end: col + span.name.length, span });
  }

  candidates.sort((a, b) => a.line - b.line || a.start - b.start);

  const decorations: DecorationItem[] = [];
  let lastLine = -1;
  let lastEnd = -1;
  for (const c of candidates) {
    if (c.line === lastLine && c.start < lastEnd) continue; // overlap — skip
    lastLine = c.line;
    lastEnd = c.end;
    decorations.push({
      start: { line: c.line, character: c.start },
      end: { line: c.line, character: c.end },
      properties: {
        class: "code-symbol",
        "data-symbol-id": c.span.id,
        "data-symbol-name": c.span.name,
        title: c.span.signature,
      },
    });
  }
  return decorations;
}

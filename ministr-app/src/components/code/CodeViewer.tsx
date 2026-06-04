/**
 * CodeViewer — renders one file with Shiki highlighting and overlays the
 * symbol index as clickable, hoverable hot-zones.
 *
 * Single responsibility: turn a {@link FileContent} into an interactive view.
 * Clicks are captured by event delegation on the container (Shiki emits the
 * `data-symbol-id` attributes via decorations); hover shows a zero-latency
 * card from the span metadata already in hand.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import type { FileContent, Occurrence, SymbolSpan } from "../../lib/types";
import { buildOccurrenceDecorations, buildSymbolDecorations } from "./decorations";
import { useHighlightedHtml } from "./useHighlighter";
import type { ColorScheme } from "./useColorScheme";
import { useDocumentScheme } from "../../hooks/useDocumentScheme";
import "./code.css";

interface Props {
  file: FileContent;
  /** Shiki colour scheme. Defaults to the live `.dark` class on <html> so the
   *  highlight always matches the surface it renders on (incl. Storybook). */
  scheme?: ColorScheme;
  /** 1-based line to scroll into view (e.g. a go-to-definition target). */
  focusLine?: number;
  /** v2 occurrence index — when non-empty, EVERY resolved token is clickable;
   *  otherwise the viewer falls back to definition name-spans. */
  occurrences?: Occurrence[];
  onSymbolClick: (symbolId: string, name: string) => void;
}

interface HoverState {
  span: SymbolSpan;
  x: number;
  y: number;
}

export function CodeViewer({
  file,
  scheme: schemeProp,
  focusLine,
  occurrences,
  onSymbolClick,
}: Props) {
  const docScheme = useDocumentScheme();
  const scheme = schemeProp ?? docScheme;
  // Prefer the v2 occurrence index (every resolved token) when present;
  // otherwise fall back to v1 definition name-spans.
  const decorations = useMemo(
    () =>
      occurrences && occurrences.length > 0
        ? buildOccurrenceDecorations(occurrences)
        : buildSymbolDecorations(file.content, file.symbol_spans),
    [occurrences, file.content, file.symbol_spans],
  );
  const { html, loading, error } = useHighlightedHtml({
    code: file.content,
    lang: file.lang,
    scheme,
    decorations,
  });

  const spanById = useMemo(() => {
    const m = new Map<string, SymbolSpan>();
    for (const s of file.symbol_spans) m.set(s.id, s);
    return m;
  }, [file.symbol_spans]);

  const containerRef = useRef<HTMLDivElement | null>(null);
  const [hover, setHover] = useState<HoverState | null>(null);

  // Keep the latest click handler in a ref so the delegation effect doesn't
  // re-bind listeners on every parent render.
  const onSymbolClickRef = useRef(onSymbolClick);
  onSymbolClickRef.current = onSymbolClick;

  // Click + hover delegation. Re-bound only when the rendered html changes.
  useEffect(() => {
    const root = containerRef.current;
    if (!root || !html) return;

    function symbolElFrom(target: EventTarget | null): HTMLElement | null {
      return (target as HTMLElement | null)?.closest<HTMLElement>(
        "[data-symbol-id]",
      ) ?? null;
    }

    function onClick(e: MouseEvent) {
      const el = symbolElFrom(e.target);
      if (!el) return;
      const id = el.getAttribute("data-symbol-id");
      const name = el.getAttribute("data-symbol-name") ?? "";
      if (id) onSymbolClickRef.current(id, name);
    }

    function onOver(e: MouseEvent) {
      const el = symbolElFrom(e.target);
      if (!el) {
        setHover(null);
        return;
      }
      const id = el.getAttribute("data-symbol-id");
      const span = id ? spanById.get(id) : undefined;
      if (!span) {
        setHover(null);
        return;
      }
      const rect = el.getBoundingClientRect();
      const hostRect = root!.getBoundingClientRect();
      setHover({
        span,
        x: rect.left - hostRect.left,
        y: rect.bottom - hostRect.top + 4,
      });
    }

    function onLeave() {
      setHover(null);
    }

    root.addEventListener("click", onClick);
    root.addEventListener("mouseover", onOver);
    root.addEventListener("mouseleave", onLeave);
    return () => {
      root.removeEventListener("click", onClick);
      root.removeEventListener("mouseover", onOver);
      root.removeEventListener("mouseleave", onLeave);
    };
  }, [html, spanById]);

  // Scroll the focus line into view, FLASH it once, and leave it SUBTLY marked
  // as the current line (so you keep your place after the flash fades). Re-runs
  // when the html re-renders (re-applies the marker) or the focus line moves.
  useEffect(() => {
    const root = containerRef.current;
    if (!root || !html) return;
    // Clear any previous persistent marker before (re)applying.
    root
      .querySelectorAll<HTMLElement>(".line.code-line-active")
      .forEach((el) => el.classList.remove("code-line-active"));
    if (!focusLine) return;
    const lines = root.querySelectorAll<HTMLElement>(".line");
    const target = lines[focusLine - 1];
    if (!target) return;
    target.classList.add("code-line-active"); // persistent current-line marker
    target.classList.add("code-line-focus"); // one-shot flash
    target.scrollIntoView({ block: "center", behavior: "smooth" });
    const id = window.setTimeout(
      () => target.classList.remove("code-line-focus"),
      1600,
    );
    return () => window.clearTimeout(id);
  }, [html, focusLine, file.path]);

  if (error) {
    return (
      <div className="grid h-full place-items-center px-6 text-center">
        <p className="font-mono text-sm text-danger">Failed to highlight: {error}</p>
      </div>
    );
  }

  if (loading || html === null) {
    return (
      <div className="grid h-full place-items-center">
        <span className="font-mono text-sm text-text-dim">Loading_</span>
      </div>
    );
  }

  return (
    <div className="relative h-full min-h-0">
      <div
        ref={containerRef}
        className="code-viewer h-full bg-surface-sunken"
        // Shiki output is sanitized HTML it generated from our text + tokens.
        dangerouslySetInnerHTML={{ __html: html }}
      />
      {hover && <Hovercard hover={hover} />}
    </div>
  );
}

function Hovercard({ hover }: { hover: HoverState }) {
  const { span, x, y } = hover;
  return (
    <div
      className="pointer-events-none absolute z-30 max-w-md rounded-md border border-border bg-surface-overlay px-3 py-2 shadow-[var(--glow-soft)]"
      style={{ left: x, top: y }}
    >
      <div className="flex items-center gap-2">
        <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
          {span.kind}
        </span>
        <span className="font-mono text-xs font-semibold text-text">{span.name}</span>
      </div>
      <div className="mt-1 font-mono text-mono-mini text-text-muted whitespace-pre-wrap">
        {span.signature}
      </div>
      {span.doc_comment && (
        <div className="mt-1.5 border-l border-accent pl-2 font-mono text-mono-mini text-text-dim whitespace-pre-wrap line-clamp-4">
          {span.doc_comment}
        </div>
      )}
    </div>
  );
}

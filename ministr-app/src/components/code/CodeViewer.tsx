/**
 * CodeViewer — renders one file with Shiki highlighting and overlays the
 * symbol index as clickable, hoverable hot-zones.
 *
 * Self-framed command-deck code surface: an identity header (file medallion +
 * basename + a divided LANG/lines/symbols vital readout + a Copy-source
 * affordance) tops a scrolling body. The header renders the instant a file is
 * in hand — before Shiki resolves — so the surface always has an identity; the
 * body swaps a premium skeleton (while highlighting), a quiet-fault panel (on
 * error), or the interactive code itself.
 *
 * Clicks are captured by event delegation on the container (Shiki emits the
 * `data-symbol-id` attributes via decorations); hover shows a zero-latency
 * card from the span metadata already in hand.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  Box,
  Braces,
  Check,
  Copy,
  FileCode2,
  Hash,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
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

  const lineCount = useMemo(
    () => file.content.replace(/\n$/, "").split("\n").length,
    [file.content],
  );

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

  return (
    <div className="flex h-full min-h-0 flex-col bg-surface-sunken">
      <CodeHeader
        file={file}
        lineCount={lineCount}
        symbolCount={file.symbol_spans.length}
      />
      <div className="relative min-h-0 flex-1">
        {error ? (
          <CodeFault message={error} />
        ) : loading || html === null ? (
          <CodeSkeleton />
        ) : (
          <>
            <div
              ref={containerRef}
              className="code-viewer h-full"
              // Shiki output is sanitized HTML it generated from our text + tokens.
              dangerouslySetInnerHTML={{ __html: html }}
            />
            {hover && <Hovercard hover={hover} />}
          </>
        )}
      </div>
    </div>
  );
}

/* ── Identity header ─────────────────────────────────────────────────────────
 * Command-deck source identity for the surface: a quiet file medallion, the
 * emphasised basename, a divided LANG/lines/symbols vital readout (tone on the
 * medallion only; the numbers stay text so they read AA), and a Copy-source
 * affordance. The full path lives in CodeBrowser's top bar — this is the
 * complementary on-surface identity, so the path is intentionally not repeated. */
function CodeHeader({
  file,
  lineCount,
  symbolCount,
}: {
  file: FileContent;
  lineCount: number;
  symbolCount: number;
}) {
  const basename = file.path.split(/[\\/]/).pop() ?? file.path;
  const [copied, setCopied] = useState(false);

  function copy() {
    void navigator.clipboard?.writeText(file.content).then(
      () => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      },
      () => {
        /* clipboard denied — leave the affordance untouched. */
      },
    );
  }

  return (
    <header className="flex shrink-0 items-center gap-3 border-b border-border-soft bg-surface px-3 py-2">
      {/* Quiet accent medallion — a code surface is present, not "live", so no glow. */}
      <span
        aria-hidden
        className="grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-accent/40 bg-surface-overlay text-accent"
      >
        <FileCode2 className="h-4 w-4" strokeWidth={2} />
      </span>
      <div className="flex min-w-0 items-center gap-3">
        <span className="truncate font-mono text-mono-mini font-semibold text-text">
          {basename}
        </span>
        {/* Divided vital readout — hairline rules, numbers tabular. */}
        <span className="hidden items-center gap-2.5 md:flex">
          <LangChip lang={file.lang} />
          <Divider />
          <Vital value={lineCount} unit={lineCount === 1 ? "line" : "lines"} />
          <Divider />
          <Vital
            value={symbolCount}
            unit={symbolCount === 1 ? "symbol" : "symbols"}
          />
        </span>
      </div>
      <button
        type="button"
        onClick={copy}
        aria-label="Copy file contents"
        className="ml-auto inline-flex shrink-0 items-center gap-1.5 rounded-md border border-border-soft px-2 py-1 font-mono text-mono-mini text-text-muted hover:border-border hover:text-text cursor-pointer transition-colors duration-150 ease-out"
      >
        {copied ? (
          <>
            <Check className="h-3 w-3 text-accent" strokeWidth={2.5} aria-hidden />
            <span>Copied</span>
          </>
        ) : (
          <>
            <Copy className="h-3 w-3" strokeWidth={2} aria-hidden />
            <span>Copy</span>
          </>
        )}
      </button>
    </header>
  );
}

function LangChip({ lang }: { lang: string }) {
  return (
    <span className="inline-flex items-center rounded border border-border-soft bg-surface px-1.5 py-0.5 font-mono text-mono-micro font-semibold uppercase tracking-[0.08em] text-text-muted">
      {lang || "text"}
    </span>
  );
}

function Divider() {
  return <span aria-hidden className="h-3 w-px bg-border-soft" />;
}

function Vital({ value, unit }: { value: number; unit: string }) {
  return (
    <span className="font-mono text-mono-mini text-text-dim">
      <span className="tabular-nums text-text-muted">{value.toLocaleString()}</span>{" "}
      {unit}
    </span>
  );
}

/* ── Body states ─────────────────────────────────────────────────────────── */

// Faux code-line widths for the loading skeleton — varied so it reads as code,
// not a paragraph block.
const SKELETON_WIDTHS = [
  "58%", "76%", "44%", "86%", "68%", "52%", "80%", "61%", "73%", "39%", "84%",
  "64%", "49%", "78%", "55%", "70%",
];

function CodeSkeleton() {
  return (
    <div className="h-full overflow-hidden py-3">
      <span className="sr-only" role="status">
        Loading file…
      </span>
      <div aria-hidden>
        {SKELETON_WIDTHS.map((w, i) => (
          <div key={i} className="flex items-center gap-4 px-3 py-1">
            <div className="h-3 w-7 shrink-0 rounded bg-surface-overlay motion-safe:animate-pulse" />
            <div
              className="h-3 rounded bg-surface-overlay motion-safe:animate-pulse"
              style={{ width: w }}
            />
          </div>
        ))}
      </div>
    </div>
  );
}

function CodeFault({ message }: { message: string }) {
  return (
    <div className="grid h-full place-items-center px-6">
      {/* Quiet fault — danger spine + no-glow danger medallion (tone discipline:
          a failure stays calm, it doesn't shout with a glow). */}
      <div className="flex max-w-md items-start gap-3 rounded-lg border-y border-r border-border-soft border-l-2 border-l-danger bg-surface px-4 py-3 shadow-sm">
        <span
          aria-hidden
          className="grid h-9 w-9 shrink-0 place-items-center rounded-lg border border-danger/40 bg-surface-overlay text-danger"
        >
          <AlertTriangle className="h-[18px] w-[18px]" strokeWidth={2} />
        </span>
        <div className="min-w-0">
          <p className="font-sans text-sm font-semibold text-text">
            Couldn’t highlight this file
          </p>
          <p className="mt-1 break-words font-mono text-mono-mini text-text-dim">
            {message}
          </p>
        </div>
      </div>
    </div>
  );
}

/* ── Hovercard ───────────────────────────────────────────────────────────── */

function kindIcon(kind: string): LucideIcon {
  const k = kind.toLowerCase();
  if (k.includes("fn") || k.includes("function") || k.includes("method")) {
    return Braces;
  }
  if (
    ["struct", "enum", "type", "trait", "interface", "class", "impl"].some((t) =>
      k.includes(t),
    )
  ) {
    return Box;
  }
  return Hash;
}

function Hovercard({ hover }: { hover: HoverState }) {
  const { span, x, y } = hover;
  const Icon = kindIcon(span.kind);
  return (
    <div
      className="pointer-events-none absolute z-30 max-w-md rounded-lg border border-border bg-surface-overlay px-3 py-2.5 shadow-[var(--glow-soft)]"
      style={{ left: x, top: y }}
    >
      <div className="flex items-center gap-2">
        <span
          aria-hidden
          className="grid h-6 w-6 shrink-0 place-items-center rounded-md border border-accent/40 bg-surface text-accent"
        >
          <Icon className="h-3.5 w-3.5" strokeWidth={2} />
        </span>
        <span className="font-mono text-mono-mini font-semibold text-text">
          {span.name}
        </span>
        <span className="ml-0.5 inline-flex items-center rounded border border-border-soft px-1.5 py-0.5 font-mono text-mono-micro font-semibold uppercase tracking-[0.08em] text-text-muted">
          {span.kind}
        </span>
      </div>
      <div className="mt-1.5 font-mono text-mono-mini text-text-muted whitespace-pre-wrap">
        {span.signature}
      </div>
      {span.doc_comment && (
        <div className="mt-1.5 border-l-2 border-accent pl-2 font-mono text-mono-mini text-text-dim whitespace-pre-wrap line-clamp-4">
          {span.doc_comment}
        </div>
      )}
    </div>
  );
}

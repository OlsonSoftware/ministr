/**
 * CodeExcerpt — a small, reusable syntax-highlighted code snippet.
 *
 * Renders a short code excerpt with real TextMate highlighting (the same
 * Shiki engine the full CodeViewer uses), the grammar inferred from a
 * filename. It preserves indentation/newlines (de-dented to the common
 * leading whitespace), clamps to `maxLines`, and sits on a transparent
 * background so it drops into any surface (source rows, citation cards,
 * search results). While the grammar loads — or for unknown languages — it
 * falls back to plain mono text, so it never blocks or throws.
 *
 *   <CodeExcerpt code={src} filename="foo/bar.rs" maxLines={3} />
 */
import { useMemo } from "react";
import { useHighlightedHtml } from "../code/useHighlighter";
import type { ColorScheme } from "../code/useColorScheme";
import { useDocumentScheme } from "../../hooks/useDocumentScheme";
import { langFromPath } from "../../lib/lang";
import { cn } from "../../lib/utils";

interface Props {
  code: string;
  /** Filename/path used to infer the Shiki grammar (ignored if `lang` set). */
  filename?: string | null;
  /** Explicit Shiki language id; overrides `filename` inference. */
  lang?: string;
  /** Clamp the excerpt to this many lines (appends an ellipsis line). */
  maxLines?: number;
  /** Colour scheme override; defaults to the live `.dark` class. */
  scheme?: ColorScheme;
  className?: string;
}

/** Strip the common leading indentation shared by every non-blank line. */
function dedent(text: string): string {
  const lines = text.split("\n");
  const indents = lines
    .filter((l) => l.trim().length > 0)
    .map((l) => (l.match(/^[ \t]*/)?.[0].length ?? 0));
  const min = indents.length ? Math.min(...indents) : 0;
  return min > 0 ? lines.map((l) => l.slice(min)).join("\n") : text;
}

export function CodeExcerpt({
  code,
  filename,
  lang,
  maxLines,
  scheme,
  className,
}: Props) {
  const docScheme = useDocumentScheme();
  const effectiveScheme = scheme ?? docScheme;
  const resolvedLang = lang ?? langFromPath(filename);

  const display = useMemo(() => {
    const trimmed = dedent(code.replace(/^\n+/, "").replace(/\s+$/, ""));
    if (!maxLines) return trimmed;
    const lines = trimmed.split("\n");
    if (lines.length <= maxLines) return trimmed;
    return lines.slice(0, maxLines).join("\n") + "\n…";
  }, [code, maxLines]);

  const { html } = useHighlightedHtml({
    code: display,
    lang: resolvedLang,
    scheme: effectiveScheme,
    decorations: [],
  });

  if (html) {
    return (
      <div
        className={cn("code-excerpt", className)}
        // Shiki output is a trusted <pre class="shiki"><code>…</code></pre>;
        // the .code-excerpt rule neutralises its inline background.
        dangerouslySetInnerHTML={{ __html: html }}
      />
    );
  }

  // Pre-highlight / unknown-grammar fallback — same formatting, no colour.
  return (
    <pre className={cn("code-excerpt code-excerpt--plain", className)}>
      {display}
    </pre>
  );
}

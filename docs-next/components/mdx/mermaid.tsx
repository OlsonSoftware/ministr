'use client';

import { useEffect, useRef, useState } from 'react';
import { useTheme } from 'next-themes';
import mermaid from 'mermaid';

export function Mermaid({ chart }: { chart: string }) {
  const { resolvedTheme } = useTheme();
  const [svg, setSvg] = useState<string>('');
  const [error, setError] = useState<string | null>(null);
  // Counter guards against StrictMode double-invocation and theme-change
  // re-renders racing each other. Each render call gets a token; only the
  // newest token is allowed to commit its result.
  const renderCounterRef = useRef(0);

  useEffect(() => {
    renderCounterRef.current += 1;
    const myToken = renderCounterRef.current;

    async function render() {
      try {
        // Fresh element id per call — mermaid.render() injects a node into
        // the document under this id and silently misbehaves on collision.
        const id = `mermaid-${myToken}-${Math.random().toString(36).slice(2, 10)}`;

        const isDark = resolvedTheme === 'dark';
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: 'loose',
          theme: isDark ? 'dark' : 'neutral',
          themeVariables: isDark
            ? {
                background: '#0a0b14',
                primaryColor: '#4338ca',
                primaryTextColor: '#e5e7eb',
                primaryBorderColor: '#4f46e5',
                lineColor: '#7c7e9a',
                secondaryColor: '#23253d',
                tertiaryColor: '#191b2f',
              }
            : {
                primaryColor: '#eef2ff',
                primaryTextColor: '#312e81',
                primaryBorderColor: '#6366f1',
                lineColor: '#78716c',
              },
          flowchart: { useMaxWidth: true, htmlLabels: true },
        });

        const { svg: renderedSvg } = await mermaid.render(id, chart);
        if (renderCounterRef.current === myToken) {
          setSvg(renderedSvg);
          setError(null);
        }
      } catch (e) {
        // Stuck "Rendering diagram…" typically means mermaid threw and we
        // swallowed it — log so it shows up in devtools.
        // eslint-disable-next-line no-console
        console.error('[Mermaid]', e);
        if (renderCounterRef.current === myToken) {
          setError((e as Error).message ?? String(e));
        }
      }
    }

    render();
  }, [chart, resolvedTheme]);

  if (error) {
    return (
      <pre className="text-[var(--color-warning)] text-xs whitespace-pre-wrap border border-fd-border rounded-lg p-3 my-4">
        Mermaid render error: {error}
        {'\n\n'}
        {chart}
      </pre>
    );
  }

  return (
    <div
      className="my-4 flex justify-center rounded-lg border border-fd-border bg-fd-card p-4 [&_svg]:max-w-full [&_svg]:h-auto"
      dangerouslySetInnerHTML={{
        __html:
          svg || '<span class="text-xs text-fd-muted-foreground">Rendering diagram…</span>',
      }}
    />
  );
}

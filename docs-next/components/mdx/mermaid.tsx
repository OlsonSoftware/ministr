'use client';

import { useEffect, useId, useRef, useState } from 'react';
import { useTheme } from 'next-themes';

export function Mermaid({ chart }: { chart: string }) {
  const { resolvedTheme } = useTheme();
  const [svg, setSvg] = useState<string>('');
  const [error, setError] = useState<string | null>(null);
  const reactId = useId().replace(/:/g, '');
  const mountedRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    async function render() {
      try {
        const { default: mermaid } = await import('mermaid');
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: 'loose',
          theme: resolvedTheme === 'dark' ? 'dark' : 'neutral',
          themeVariables:
            resolvedTheme === 'dark'
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
        const { svg } = await mermaid.render(`mermaid-${reactId}`, chart);
        if (!cancelled) setSvg(svg);
      } catch (e) {
        if (!cancelled) setError((e as Error).message);
      }
    }
    render();
    mountedRef.current = true;
    return () => {
      cancelled = true;
    };
  }, [chart, resolvedTheme, reactId]);

  if (error) {
    return (
      <pre className="text-[var(--color-warning)] text-xs whitespace-pre-wrap border border-fd-border rounded-lg p-3">
        Mermaid render error: {error}
        {'\n\n'}
        {chart}
      </pre>
    );
  }

  return (
    <div
      className="my-4 flex justify-center rounded-lg border border-fd-border bg-fd-card p-4 [&_svg]:max-w-full [&_svg]:h-auto"
      // mermaid returns already-sanitized SVG; rendering as innerHTML is standard pattern
      dangerouslySetInnerHTML={{ __html: svg || '<span class="text-xs text-fd-muted-foreground">Rendering diagram…</span>' }}
    />
  );
}

import Link from 'next/link';

const BRIDGES = [
  {
    kind: 'napi',
    rust: '#[napi]\nfn greet(s: String)',
    consumer: "import { greet }\nfrom './native'",
  },
  {
    kind: 'pyo3',
    rust: '#[pyfunction]\nfn compute(x: f64)',
    consumer: 'from mylib import\n    compute',
  },
  {
    kind: 'tauri',
    rust: '#[tauri::command]\nfn open_file(path)',
    consumer: "invoke('open_file',\n   { path })",
  },
];

export function BridgesDiagram() {
  return (
    <div className="mx-auto w-full max-w-5xl px-4 sm:px-6">
      <div className="rounded-2xl border border-fd-border bg-fd-card p-6 sm:p-8">
        <div className="mb-6 flex items-center justify-between text-xs font-semibold uppercase tracking-wider text-fd-muted-foreground">
          <span>Rust</span>
          <span>JavaScript / Python</span>
        </div>
        <div className="space-y-3">
          {BRIDGES.map((bridge) => (
            <div
              key={bridge.kind}
              className="grid items-center gap-3 sm:grid-cols-[1fr_auto_1fr]"
            >
              <pre className="rounded-lg border border-fd-border bg-fd-background p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap">
                {bridge.rust}
              </pre>
              <div className="flex items-center justify-center gap-2">
                <svg
                  width="32"
                  height="24"
                  viewBox="0 0 32 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  className="text-[var(--color-iris-500)]"
                  aria-hidden
                >
                  <path
                    d="M4 12h24M22 6l6 6-6 6"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
                <span className="rounded-full border border-fd-border bg-fd-background px-2 py-0.5 font-mono text-[10px] text-fd-muted-foreground">
                  {bridge.kind}
                </span>
              </div>
              <pre className="rounded-lg border border-fd-border bg-fd-background p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap">
                {bridge.consumer}
              </pre>
            </div>
          ))}
        </div>
      </div>
      <p className="mt-3 text-center text-sm text-fd-muted-foreground">
        Query these links with{' '}
        <Link href="/docs/tools/bridge" className="font-mono underline-offset-4 hover:underline">
          iris_bridge
        </Link>
        .
      </p>
    </div>
  );
}

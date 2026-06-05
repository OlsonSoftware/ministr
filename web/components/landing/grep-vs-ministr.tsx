/**
 * GrepVsMinistr — the homepage thesis, made visual.
 *
 * The page claims agents get answers that are "structurally correct instead of
 * grep-approximated." This figure SHOWS it: the same question, asked two ways.
 * Left, grep returns a wall of textual matches with no idea which is the
 * definition. Right, ministr returns the symbol itself — signature, callers,
 * references, and the cross-language call grep can never see.
 *
 * Static server component. No client JS, no motion, no decoration — it earns
 * its place by demonstrating the product, in the page's own v2 language
 * (warm ink, single amber accent, sharp corners, hairline rules, mono).
 */

// A representative grep dump: the real definition is buried among a comment, a
// log string, a doc reference, a test, and an unrelated same-named symbol.
const GREP_LINES: Array<{ path: string; ln: string; text: string }> = [
  { path: 'src/net/server.rs', ln: '142', text: 'fn handle_event(&self, ev: Event) -> Result<()> {' },
  { path: 'src/net/server.rs', ln: '88', text: '// handle_event is called on every inbound frame' },
  { path: 'src/net/router.rs', ln: '210', text: 'self.handle_event(ev).await?;' },
  { path: 'src/log.rs', ln: '54', text: 'tracing::debug!("handle_event: {kind:?}");' },
  { path: 'docs/internals.md', ln: '31', text: 'The `handle_event` hook fans out to subscribers.' },
  { path: 'tests/wire.rs', ln: '405', text: 'fn handle_event_roundtrip() {' },
  { path: 'src/ui/menu.rs', ln: '77', text: 'fn handle_event(&mut self, _: MenuEvent) {} // unrelated' },
  { path: 'vendor/legacy.rs', ln: '1190', text: '#[deprecated] fn handle_event_v1() {}' },
];

const FACTS: Array<{ k: string; v: React.ReactNode }> = [
  { k: 'callers', v: '3 — router::dispatch, retry_loop, tests::roundtrip' },
  { k: 'references', v: '11 across 4 files' },
  {
    k: 'cross-language',
    v: (
      <>
        <span className="v2-demo-bridge">PyO3</span> → pyhost.on_event
        <span className="v2-demo-lang"> (python)</span>
      </>
    ),
  },
];

export function GrepVsMinistr() {
  return (
    <figure
      className="v2-demo"
      aria-label="The same question — locate handle_event — answered by grep versus ministr."
    >
      {/* ── grep: text matching ─────────────────────────────── */}
      <div className="v2-demo-col">
        <div className="v2-demo-head">
          <span className="v2-demo-tool">grep</span>
          <span className="v2-demo-note">text match · 8 of ~200 hits</span>
        </div>
        <div className="v2-demo-cmd">grep -rn &quot;handle_event&quot; .</div>
        <ul className="v2-demo-grep">
          {GREP_LINES.map((l, i) => (
            <li key={i}>
              <span className="v2-demo-loc">
                {l.path}:{l.ln}
              </span>
              <span className="v2-demo-hit">{l.text}</span>
            </li>
          ))}
        </ul>
        <p className="v2-demo-verdict">
          Which line is the definition? Which calls it? grep can&apos;t tell you.
        </p>
      </div>

      {/* ── ministr: structural ─────────────────────────────── */}
      <div className="v2-demo-col v2-demo-col-answer">
        <div className="v2-demo-head">
          <span className="v2-demo-tool v2-demo-tool-amber">ministr</span>
          <span className="v2-demo-note">structural · the answer</span>
        </div>
        <div className="v2-demo-cmd v2-demo-cmd-amber">
          ministr_definition handle_event
        </div>
        <div className="v2-demo-sym">
          <code className="v2-demo-sig">
            <span className="v2-demo-kw">fn </span>
            <span className="v2-demo-name">handle_event</span>
            (&amp;self, ev: <span className="v2-demo-ty">Event</span>){' '}
            -&gt; <span className="v2-demo-ty">Result</span>
          </code>
          <div className="v2-demo-symloc">src/net/server.rs:142 · pub</div>
        </div>
        <dl className="v2-demo-facts">
          {FACTS.map((f) => (
            <div key={f.k} className="v2-demo-fact">
              <dt>{f.k}</dt>
              <dd>{f.v}</dd>
            </div>
          ))}
        </dl>
        <p className="v2-demo-verdict v2-demo-verdict-ok">
          One symbol — its signature, its callers, and the Python that calls it.
        </p>
      </div>
    </figure>
  );
}

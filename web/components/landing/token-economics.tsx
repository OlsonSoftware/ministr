/**
 * TokenEconomics — the "fewer tokens" claim, measured.
 *
 * Not illustrative, not a flat constant. These are the real end-to-end numbers
 * from web/content/docs/benchmarks.mdx (produced by
 * ministr-mcp/tests/token_economics_e2e.rs): index a real multi-language corpus,
 * run real `ministr_survey` calls through the MCP interface, and count the
 * literal response bytes against a grep + read of the candidate files. Across 26
 * real queries a ministr lookup averaged 518 tokens vs 1,548 for grep + read —
 * 66% fewer, and cheaper on 23 of the 26. A lookup is bounded, not free; the
 * advantage widens as a repo grows (see the docs for the scaling model).
 *
 * Static, in the page's v2 language (warm ink, single amber, hairline, mono).
 */
import Link from 'next/link';

const GREP_TOKENS = 1_548;
const MINISTR_TOKENS = 518;
// Floor, not round — never round a savings claim up (66.5% measured → 66%).
const REDUCTION = Math.floor((1 - MINISTR_TOKENS / GREP_TOKENS) * 100); // 66

interface Row {
  label: string;
  tokens: number;
  cut?: string;
}

// Mean tokens per lookup, measured. The track is scaled to grep+read = 100%.
const ROWS: Row[] = [
  { label: 'grep+read', tokens: GREP_TOKENS },
  { label: 'ministr', tokens: MINISTR_TOKENS, cut: `−${REDUCTION}%` },
];

const MAX = Math.max(...ROWS.map((r) => r.tokens));

export function TokenEconomics() {
  return (
    <figure
      className="v2-tok"
      aria-label={`Mean tokens per lookup, measured across 26 real queries on a real multi-language codebase. A ministr_survey lookup averages ${MINISTR_TOKENS} tokens; reading the candidate files grep surfaces averages ${GREP_TOKENS.toLocaleString()} — a ${REDUCTION}% reduction, with ministr cheaper on 23 of the 26 queries.`}
    >
      <div className="v2-tok-head">
        <span className="v2-tok-eyebrow">Tokens per lookup · measured</span>
        <span className="v2-tok-flat">
          <b>{REDUCTION}%</b> fewer, on 23 of 26 real queries
        </span>
      </div>

      <ul className="v2-tok-rows">
        {ROWS.map((r) => (
          <li key={r.label} className="v2-tok-row">
            <span className="v2-tok-label">{r.label}</span>
            <span className="v2-tok-track">
              <span
                className="v2-tok-fill"
                style={{ width: `${(r.tokens / MAX) * 100}%` }}
              >
                <span className="v2-tok-count">{r.tokens.toLocaleString()}</span>
              </span>
            </span>
            <span className="v2-tok-cut">{r.cut ?? ''}</span>
          </li>
        ))}
      </ul>

      <figcaption className="v2-tok-cap">
        Measured end-to-end: a real corpus indexed, real <code>ministr_survey</code>{' '}
        calls run, the literal response counted against reading the files grep
        surfaces — same tokenizer ministr uses internally. A lookup is bounded,
        not free, and the gap widens with repo size.{' '}
        <Link href="/docs/benchmarks" className="v2-tok-link">
          See the benchmark →
        </Link>
      </figcaption>
    </figure>
  );
}

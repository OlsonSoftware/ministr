/**
 * TokenEconomics — the "90% fewer tokens" claim, proven.
 *
 * The Why section asserts the headline number; this shows the measured shape
 * behind it. ministr returns the same targeted slice every time (a constant 68
 * tokens), while the grep + read workflow's cost scales with how many candidate
 * files the agent has to read to be sure. The grep bars grow; ministr's
 * baseline stays flat.
 *
 * Numbers are the real benchmark from web/content/docs/benchmarks.mdx (counted
 * with ministr's own tokenizer) — not illustrative. Static, in the page's v2
 * language (warm ink, single amber, sharp corners, hairline, mono).
 */
import Link from 'next/link';

const MINISTR_TOKENS = 68;

interface Row {
  files: number;
  grep: number;
  reduction: string;
}

// web/content/docs/benchmarks.mdx — grep+read tokens vs the flat 68-token slice.
const ROWS: Row[] = [
  { files: 5, grep: 1_215, reduction: '94.4%' },
  { files: 20, grep: 4_860, reduction: '98.6%' },
  { files: 50, grep: 12_150, reduction: '99.4%' },
  { files: 100, grep: 24_300, reduction: '99.7%' },
];

const MAX = Math.max(...ROWS.map((r) => r.grep));

export function TokenEconomics() {
  return (
    <figure
      className="v2-tok"
      aria-label={`Token cost per lookup, measured. ministr returns a constant ${MINISTR_TOKENS} tokens; the grep-and-read workflow grows from ${ROWS[0].grep.toLocaleString()} tokens across 5 candidate files to ${ROWS[ROWS.length - 1].grep.toLocaleString()} across 100 — a ${ROWS[ROWS.length - 1].reduction} reduction at the high end.`}
    >
      <div className="v2-tok-head">
        <span className="v2-tok-eyebrow">Tokens per lookup · measured</span>
        <span className="v2-tok-flat">
          ministr: a flat <b>{MINISTR_TOKENS}</b> tokens, every time
        </span>
      </div>

      <ul className="v2-tok-rows">
        {ROWS.map((r) => (
          <li key={r.files} className="v2-tok-row">
            <span className="v2-tok-label">{r.files} files</span>
            <span className="v2-tok-track">
              <span
                className="v2-tok-fill"
                style={{ width: `${(r.grep / MAX) * 100}%` }}
              >
                <span className="v2-tok-count">{r.grep.toLocaleString()}</span>
              </span>
            </span>
            <span className="v2-tok-cut">−{r.reduction}</span>
          </li>
        ))}
      </ul>

      <figcaption className="v2-tok-cap">
        grep + read grows with every candidate file the agent must scan; the
        targeted slice stays flat. Same tokenizer ministr uses internally.{' '}
        <Link href="/docs/benchmarks" className="v2-tok-link">
          See the benchmark →
        </Link>
      </figcaption>
    </figure>
  );
}

// F3.6-c-i — filter UI for the bridge-graph visualizer.
//
// Three controls:
// 1. Language chips — multi-select from the distinct languages in
//    the current data. Click to toggle. None selected = admit all.
// 2. Bridge-kind chips — same shape, derived from the distinct edge
//    kinds.
// 3. File substring — case-insensitive text input. Empty = admit
//    all.
//
// Pure presentational: takes the current filter state + callbacks +
// the derived chip universes; does no fetching of its own. Lives
// alongside the live wrapper which holds the actual state.

'use client';

import type { BridgeFilters } from './bridge-filters';

interface BridgeGraphFiltersProps {
  filters: BridgeFilters;
  onChange: (next: BridgeFilters) => void;
  /** Distinct language slugs available in the current data. */
  availableLanguages: ReadonlyArray<string>;
  /** Distinct bridge-kind slugs available in the current data. */
  availableKinds: ReadonlyArray<string>;
}

export function BridgeGraphFilters({
  filters,
  onChange,
  availableLanguages,
  availableKinds,
}: BridgeGraphFiltersProps) {
  function toggleLang(lang: string) {
    const next = new Set(filters.languages);
    if (next.has(lang)) next.delete(lang);
    else next.add(lang);
    onChange({ ...filters, languages: next });
  }
  function toggleKind(kind: string) {
    const next = new Set(filters.kinds);
    if (next.has(kind)) next.delete(kind);
    else next.add(kind);
    onChange({ ...filters, kinds: next });
  }
  function setFileSubstring(value: string) {
    onChange({ ...filters, fileSubstring: value });
  }
  function clearAll() {
    onChange({ languages: new Set(), kinds: new Set(), fileSubstring: '' });
  }

  const anyActive =
    filters.languages.size > 0 || filters.kinds.size > 0 || filters.fileSubstring.trim() !== '';

  return (
    <div className="mb-4 flex flex-col gap-3 rounded border border-fd-border bg-fd-muted/30 p-3 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-xs uppercase tracking-wide text-fd-muted-foreground">
          Languages
        </span>
        {availableLanguages.length === 0 ? (
          <span className="text-fd-muted-foreground">—</span>
        ) : (
          availableLanguages.map((lang) => (
            <Chip
              key={lang}
              label={lang}
              active={filters.languages.has(lang)}
              onClick={() => toggleLang(lang)}
            />
          ))
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-xs uppercase tracking-wide text-fd-muted-foreground">
          Bridge kinds
        </span>
        {availableKinds.length === 0 ? (
          <span className="text-fd-muted-foreground">—</span>
        ) : (
          availableKinds.map((kind) => (
            <Chip
              key={kind}
              label={kind}
              active={filters.kinds.has(kind)}
              onClick={() => toggleKind(kind)}
            />
          ))
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <label
          htmlFor="bridge-filter-file"
          className="font-mono text-xs uppercase tracking-wide text-fd-muted-foreground"
        >
          File contains
        </label>
        <input
          id="bridge-filter-file"
          type="text"
          value={filters.fileSubstring}
          onChange={(e) => setFileSubstring(e.target.value)}
          placeholder="e.g. src-tauri or commands_cloud"
          className="flex-1 min-w-[12rem] rounded border border-fd-border bg-fd-background px-2 py-1 font-mono text-xs"
        />
        {anyActive ? (
          <button
            type="button"
            onClick={clearAll}
            className="rounded border border-fd-border px-2 py-1 text-xs hover:bg-fd-muted"
          >
            Clear filters
          </button>
        ) : null}
      </div>
    </div>
  );
}

interface ChipProps {
  label: string;
  active: boolean;
  onClick: () => void;
}

function Chip({ label, active, onClick }: ChipProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={[
        'rounded-full border px-2 py-0.5 font-mono text-xs transition',
        active
          ? 'border-emerald-500/60 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300'
          : 'border-fd-border text-fd-muted-foreground hover:bg-fd-muted',
      ].join(' ')}
    >
      {label}
    </button>
  );
}

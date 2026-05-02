import { ChevronRight, X } from "lucide-react";
import {
  entityKindLabel,
  entityLabel,
  useEntityPanel,
  type Entity,
} from "../hooks/useEntityPanel";
import { cn } from "../lib/utils";
import { SymbolView } from "./entity/SymbolView";
import { SectionView } from "./entity/SectionView";
import { BridgeView } from "./entity/BridgeView";
import { FileView } from "./entity/FileView";
import { SessionView } from "./entity/SessionView";
import { CorpusView } from "./entity/CorpusView";

/**
 * Universal entity-detail drawer. Slides in from the right; on narrow
 * windows takes the full screen. Brutalist 2px border + hard accent shadow.
 *
 * The panel is rendered at the App.tsx root so any page can call
 * `openEntity()` from anywhere via `useEntityPanel()`.
 */
export function EntityPanel() {
  const { current, stack, popTo, close } = useEntityPanel();

  if (!current) return null;

  const trail: Entity[] = [...stack, current];

  return (
    <>
      {/* Backdrop — click to close. */}
      <div
        className="fixed inset-0 z-[1200] bg-black/40"
        onClick={close}
        aria-hidden="true"
      />

      {/* Drawer — the one signature shadow on screen when open. */}
      <aside
        className={cn(
          "fixed top-0 right-0 bottom-0 z-[1201] bg-surface flex flex-col",
          "border-l-2 border-border shadow-[var(--shadow-lg)]",
          // Wide: ~58% width capped at 1200px. Narrow: full screen.
          "w-full @max-[1023px]/page:w-full",
          "min-[1024px]:w-[clamp(720px,58vw,1200px)]",
        )}
        role="dialog"
        aria-modal="true"
        aria-label="Entity detail"
      >
        {/* Header — breadcrumbs + close. Hairline below, no 2px frame. */}
        <header className="flex items-center gap-3 border-b border-border-soft bg-surface-overlay px-4 py-2.5 shrink-0">
          <div className="flex items-center gap-1.5 flex-wrap min-w-0 flex-1">
            {trail.map((e, i) => {
              const isLast = i === trail.length - 1;
              return (
                <div key={i} className="flex items-center gap-1.5 min-w-0">
                  <button
                    onClick={() => popTo(i)}
                    disabled={isLast}
                    className={cn(
                      "inline-flex items-baseline gap-1.5 px-1 py-0.5 font-mono text-xs transition-none",
                      isLast
                        ? "text-text font-bold border-b-2 border-accent cursor-default"
                        : "text-text-muted hover:text-text border-b border-transparent hover:border-border cursor-pointer",
                    )}
                  >
                    <span className="text-text-dim text-[0.6875rem] uppercase tracking-[0.05em]">
                      {entityKindLabel(e)}
                    </span>
                    <span className="truncate max-w-[200px]">
                      {entityLabel(e)}
                    </span>
                  </button>
                  {!isLast && (
                    <ChevronRight
                      className="h-3 w-3 text-text-dim shrink-0"
                      strokeWidth={2}
                    />
                  )}
                </div>
              );
            })}
          </div>
          <button
            onClick={close}
            aria-label="Close panel"
            title="Close · Esc"
            className="grid h-7 w-7 shrink-0 place-items-center border border-border bg-surface text-text-muted hover:text-text hover:border-border-hover cursor-pointer transition-none"
            style={{ borderRadius: "var(--radius-button)" }}
          >
            <X className="h-3.5 w-3.5" strokeWidth={2} />
          </button>
        </header>

        {/* Body — dispatch on entity kind */}
        <div className="flex-1 min-h-0 overflow-y-auto px-4 py-4">
          <EntityBody entity={current} />
        </div>
      </aside>
    </>
  );
}

function EntityBody({ entity }: { entity: Entity }) {
  switch (entity.kind) {
    case "symbol":
      return <SymbolView entity={entity} />;
    case "section":
      return <SectionView entity={entity} />;
    case "bridge":
      return <BridgeView entity={entity} />;
    case "file":
      return <FileView entity={entity} />;
    case "session":
      return <SessionView entity={entity} />;
    case "corpus":
      return <CorpusView entity={entity} />;
  }
}

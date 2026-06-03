import { useCallback, useRef, useState } from "react";
import { ChevronRight, X } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import {
  entityKindLabel,
  entityLabel,
  useEntityPanel,
  type Entity,
} from "../hooks/useEntityPanel";
import { scrim, slideOver, spring } from "../lib/motion";
import { overlayScrim } from "../lib/ui-tokens";
import { useDialog } from "../hooks/useDialog";
import { cn } from "../lib/utils";
import { SymbolView } from "./entity/SymbolView";
import { SectionView } from "./entity/SectionView";
import { BridgeView } from "./entity/BridgeView";
import { FileView } from "./entity/FileView";
import { SessionView } from "./entity/session/SessionView";
import { CorpusView } from "./entity/CorpusView";

const MIN_W = 560;
const MAX_W = 1280;
const DEFAULT_W = 760;

/**
 * Universal entity inspector. Spring slide-over from the right; resizable
 * by dragging its left edge; full-screen on narrow windows. Stacked
 * navigation via `useEntityPanel` (unchanged API).
 */
export function EntityPanel() {
  const { current, stack, popTo, close } = useEntityPanel();
  const [width, setWidth] = useState(DEFAULT_W);
  const dragging = useRef(false);
  // Escape now actually closes (the header's "Close · Esc" affordance
  // was a lie), focus enters the panel and is restored on close, and
  // Tab is trapped inside it.
  const panelRef = useDialog<HTMLElement>(Boolean(current), close);

  const onResizeStart = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      dragging.current = true;
      const startX = e.clientX;
      const startW = width;
      const onMove = (ev: MouseEvent) => {
        if (!dragging.current) return;
        const next = Math.min(
          MAX_W,
          Math.max(MIN_W, startW + (startX - ev.clientX)),
        );
        setWidth(next);
      };
      const onUp = () => {
        dragging.current = false;
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [width],
  );

  const trail: Entity[] = current ? [...stack, current] : [];

  return (
    <AnimatePresence>
      {current && (
        <>
          <motion.div
            key="scrim"
            variants={scrim}
            initial="initial"
            animate="animate"
            exit="exit"
            className={cn(overlayScrim, "z-[1200]")}
            onClick={close}
            aria-hidden="true"
          />

          <motion.aside
            key="panel"
            ref={panelRef}
            variants={slideOver}
            initial="initial"
            animate="animate"
            exit="exit"
            style={{ width: `min(100vw, ${width}px)` }}
            className={cn(
              "fixed top-0 right-0 bottom-0 z-[1201] bg-surface flex flex-col",
              "border-l border-border shadow-lg",
            )}
            role="dialog"
            aria-modal="true"
            aria-label="Entity detail"
          >
            {/* Resize handle — left edge. */}
            <div
              onMouseDown={onResizeStart}
              className="workspace-resizer absolute left-0 top-0 bottom-0 -ml-[3px] hidden min-[900px]:block"
              aria-hidden="true"
            />

            <header className="flex items-center gap-3 border-b border-border bg-surface-overlay px-4 py-2.5 shrink-0">
              <div className="flex items-center gap-1 flex-wrap min-w-0 flex-1">
                {trail.map((e, i) => {
                  const isLast = i === trail.length - 1;
                  return (
                    <div
                      key={i}
                      className="flex items-center gap-1 min-w-0"
                    >
                      <motion.button
                        layout
                        onClick={() => popTo(i)}
                        disabled={isLast}
                        className={cn(
                          "inline-flex items-baseline gap-1.5 px-1.5 py-1 rounded-md font-mono text-xs",
                          "transition-colors duration-150",
                          isLast
                            ? "text-text font-semibold bg-surface cursor-default"
                            : "text-text-muted hover:text-text hover:bg-surface cursor-pointer",
                        )}
                      >
                        <span className="text-text-dim text-mono-mini uppercase tracking-[0.08em]">
                          {entityKindLabel(e)}
                        </span>
                        <span className="truncate max-w-[220px]">
                          {entityLabel(e)}
                        </span>
                      </motion.button>
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
                className="grid h-7 w-7 shrink-0 place-items-center rounded-md border border-border bg-surface text-text-muted hover:text-text hover:border-border-hover cursor-pointer transition-colors duration-150"
              >
                <X className="h-3.5 w-3.5" strokeWidth={2} />
              </button>
            </header>

            <div className="flex-1 min-h-0 overflow-y-auto px-5 py-5">
              <AnimatePresence mode="wait">
                <motion.div
                  key={trail.length}
                  initial={{ opacity: 0, x: 16 }}
                  animate={{ opacity: 1, x: 0 }}
                  exit={{ opacity: 0, x: -16 }}
                  transition={spring}
                >
                  <EntityBody entity={current} />
                </motion.div>
              </AnimatePresence>
            </div>
          </motion.aside>
        </>
      )}
    </AnimatePresence>
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

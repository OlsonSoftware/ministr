import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import type {
  BridgeLink,
  CorpusInfo,
  SearchResult,
  SymbolInfo,
} from "../lib/types";

/**
 * Universal entity descriptor — every "thing" in the app the user can
 * inspect. The EntityPanel routes on `kind` to render the appropriate view.
 */
export type Entity =
  | { kind: "symbol"; corpusId: string; symbol: SymbolInfo }
  | { kind: "section"; corpusId: string; result: SearchResult }
  | { kind: "bridge"; corpusId: string; link: BridgeLink }
  | { kind: "file"; corpusId: string; path: string }
  | { kind: "session"; corpusId: string; sessionId: string }
  | { kind: "corpus"; corpus: CorpusInfo };

/** Short label used in the breadcrumb back-stack. */
export function entityLabel(e: Entity): string {
  switch (e.kind) {
    case "symbol":
      return e.symbol.name;
    case "section": {
      const id = e.result.content_id.replace(/\\/g, "/");
      return id.split("/").pop() ?? id;
    }
    case "bridge":
      return `${e.link.export_symbol || e.link.export_binding_key} ↔ ${
        e.link.import_symbol || e.link.import_binding_key
      }`;
    case "file":
      return e.path.split(/[\\/]/).pop() ?? e.path;
    case "session":
      return e.sessionId.slice(0, 8);
    case "corpus":
      return e.corpus.id;
  }
}

export function entityKindLabel(e: Entity): string {
  switch (e.kind) {
    case "symbol":
      return e.symbol.kind.toUpperCase();
    case "section":
      return "SECTION";
    case "bridge":
      return e.link.kind.toUpperCase();
    case "file":
      return "FILE";
    case "session":
      return "SESSION";
    case "corpus":
      return "CORPUS";
  }
}

interface EntityPanelContextShape {
  /** Currently visible entity, if any. */
  current: Entity | null;
  /** Back-stack — entries before `current`. */
  stack: Entity[];
  /** Push a new entity (descends one level). */
  openEntity: (entity: Entity) => void;
  /** Pop back N levels. */
  popTo: (index: number) => void;
  /** Close the panel and clear the stack. */
  close: () => void;
}

const EntityPanelContext = createContext<EntityPanelContextShape | null>(null);

interface ProviderProps {
  children: ReactNode;
}

export function EntityPanelProvider({ children }: ProviderProps) {
  // Combined "all entities visited in this opening" — last item is current.
  const [trail, setTrail] = useState<Entity[]>([]);

  const openEntity = useCallback((entity: Entity) => {
    setTrail((prev) => [...prev, entity]);
  }, []);

  const popTo = useCallback((index: number) => {
    setTrail((prev) => prev.slice(0, index + 1));
  }, []);

  const close = useCallback(() => setTrail([]), []);

  // Esc closes when panel is open. Listed here so any page hosting the
  // provider gets it for free.
  useEffect(() => {
    if (trail.length === 0) return;
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      const typing =
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable;
      if (typing) return;
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [trail.length, close]);

  const value = useMemo<EntityPanelContextShape>(() => {
    return {
      current: trail.length > 0 ? trail[trail.length - 1] : null,
      stack: trail.slice(0, -1),
      openEntity,
      popTo,
      close,
    };
  }, [trail, openEntity, popTo, close]);

  return createElement(EntityPanelContext.Provider, { value }, children);
}

/** Hook accessor; returns no-op when used outside a provider. */
export function useEntityPanel(): EntityPanelContextShape {
  const ctx = useContext(EntityPanelContext);
  return (
    ctx ?? {
      current: null,
      stack: [],
      openEntity: () => {},
      popTo: () => {},
      close: () => {},
    }
  );
}

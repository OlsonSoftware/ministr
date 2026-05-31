/**
 * Back-stack navigation for the Code surface.
 *
 * A single responsibility: own the stack of visited locations and the
 * push/back/reset transitions. The browser reads `current` to decide which
 * file to load and where to focus; "go to definition" and reference jumps
 * `push`, the breadcrumb `back`s, and switching corpus `reset`s.
 */
import { useCallback, useState } from "react";

export interface CodeLocation {
  /** Stored file path (as returned by `list_corpus_files` / `read_file`). */
  path: string;
  /** Optional 1-based line to scroll to once the file is loaded. */
  line?: number;
  /** Optional symbol id to focus (resolved to a line via the file's spans). */
  symbolId?: string;
}

export interface CodeNavigation {
  current: CodeLocation | null;
  stack: readonly CodeLocation[];
  canBack: boolean;
  push: (loc: CodeLocation) => void;
  back: () => void;
  reset: (loc?: CodeLocation) => void;
}

export function useCodeNavigation(initial?: CodeLocation): CodeNavigation {
  const [stack, setStack] = useState<CodeLocation[]>(initial ? [initial] : []);

  const push = useCallback((loc: CodeLocation) => {
    setStack((s) => {
      const top = s[s.length - 1];
      // Collapse a no-op re-push of the same path+line.
      if (top && top.path === loc.path && top.line === loc.line) return s;
      return [...s, loc];
    });
  }, []);

  const back = useCallback(() => {
    setStack((s) => (s.length > 1 ? s.slice(0, -1) : s));
  }, []);

  const reset = useCallback((loc?: CodeLocation) => {
    setStack(loc ? [loc] : []);
  }, []);

  return {
    current: stack[stack.length - 1] ?? null,
    stack,
    canBack: stack.length > 1,
    push,
    back,
    reset,
  };
}

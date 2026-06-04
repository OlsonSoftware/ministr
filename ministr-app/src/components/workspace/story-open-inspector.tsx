import { useEffect } from "react";
import { useEntityPanel } from "../../hooks/useEntityPanel";
import type { SessionDetail } from "../../lib/types";

/**
 * Test-only helper: opens a session in the shared EntityPanel on mount, so a
 * Storybook `play` function can start from the rendered `SessionView` inspector
 * (the cross-facet code-touched→Explore jump e2e).
 *
 * It lives in a normal module — NOT the `.stories.tsx` — on purpose: a
 * hook-calling component defined inside a story file is instrumented by the
 * Storybook CSF transform instead of the standard React Compiler pass, which
 * leaves its `useMemoCache` call with a null dispatcher ("invalid hook call").
 * Compiled here, its hooks run normally.
 */
export function OpenSessionInspector({ session }: { session: SessionDetail }) {
  const { openEntity } = useEntityPanel();
  useEffect(() => {
    openEntity({
      kind: "session",
      corpusId: session.corpus_id,
      sessionId: session.session_id,
      seed: session,
    });
  }, [openEntity, session]);
  return null;
}

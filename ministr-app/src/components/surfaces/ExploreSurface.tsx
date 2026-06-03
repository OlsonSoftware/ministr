import type { DaemonStatus } from "../../lib/types";
import { CodeBrowser } from "../code/CodeBrowser";

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
}

/**
 * The Explore facet — the code index, flat. No nested sub-sidebar: Explore IS
 * the CodeBrowser (click-any-token source browser / symbol graph / bridges).
 * Server + Logs moved to the Tend facet (system care), per the OOUX
 * "no nested chrome inside a facet" rule.
 */
export function ExploreSurface({ status, activeCorpusId }: Props) {
  return <CodeBrowser status={status} activeCorpusId={activeCorpusId} />;
}

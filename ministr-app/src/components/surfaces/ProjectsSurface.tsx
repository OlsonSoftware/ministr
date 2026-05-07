import type { CorpusInfo } from "../../lib/types";
import { ProjectList } from "../ProjectList";

interface Props {
  corpora: CorpusInfo[];
  activeCorpusId: string | null;
  onSelectCorpus: (id: string) => void;
  onRefresh: () => void;
}

/**
 * Projects surface — top-level destination for managing indexed projects.
 *
 * In M1 this wraps the existing ProjectList component so the migration
 * doesn't lose features (multi-select detection, reindex/remove dialogs,
 * progress visualisation). M2 will rebuild ProjectList itself with a
 * single confirmation pattern, multi-select add, and the new
 * indexing-progress event stream.
 */
export function ProjectsSurface({
  corpora,
  activeCorpusId,
  onSelectCorpus,
  onRefresh,
}: Props) {
  return (
    <div className="h-full overflow-y-auto p-5">
      <ProjectList
        corpora={corpora}
        onRefresh={onRefresh}
        onSelect={onSelectCorpus}
        selectedId={activeCorpusId}
      />
    </div>
  );
}

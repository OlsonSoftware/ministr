import type { CorpusInfo } from "./types";

/** Display label for a corpus. Always prefers the daemon-supplied
 *  `display_name` (LCA basename of the registered paths) and falls
 *  back, in order, to the LCA basename of `paths`, the basename of
 *  the first path, or finally the corpus id. The id is the last
 *  resort so the UI never silently shows `multi-######` to a human. */
export function corpusLabel(corpus: CorpusInfo | null | undefined): string {
  if (!corpus) return "Unknown corpus";
  const supplied = corpus.display_name?.trim();
  if (supplied) return supplied;
  const root = corpusRoot(corpus.paths);
  if (!root) return corpus.id;
  const parts = root.split(/[\\/]/);
  return parts[parts.length - 1] || parts[parts.length - 2] || corpus.id;
}

/** Resolve a corpus id against a list of corpora and return the label.
 *  Used by surfaces (session cards, ingestion progress, status feeds)
 *  that hold only an id and need to look up the rest. */
export function corpusLabelById(
  corpora: readonly CorpusInfo[] | null | undefined,
  id: string,
): string {
  return corpusLabel(corpora?.find((c) => c.id === id));
}

/** Longest common ancestor directory of a path set. Exposed for
 *  surfaces that want a path-style subtitle (e.g. the project-list
 *  row's secondary line) without re-implementing the LCA logic. */
export function corpusRoot(paths: readonly string[]): string {
  if (!paths.length) return "";
  if (paths.length === 1) return paths[0].replace(/[\\/]+$/, "");
  const segments = paths.map((p) => p.split(/[\\/]/));
  let common = 0;
  outer: for (let i = 0; i < segments[0].length; i++) {
    for (let j = 1; j < segments.length; j++) {
      if (i >= segments[j].length || segments[j][i] !== segments[0][i]) break outer;
    }
    common = i + 1;
  }
  return segments[0].slice(0, common).join("/");
}

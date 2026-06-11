import type { FileFreshness, FreshnessResponse } from "./ipc";
import type { TrustState } from "../components/ui/trust";

/**
 * Derivation layer: freshness facts → the plain-English trust summary
 * (DESIGN §8 register). Every sentence here restates counts the daemon
 * measured — no embellishment.
 */

export interface TrustSummary {
  state: TrustState;
  headline: string;
  sub: string;
  /** Files the AI can't currently mirror (stale + new). */
  behindCount: number;
}

export function summarize(name: string, fresh: FreshnessResponse): TrustSummary {
  const stale = fresh.files.filter((f) => f.state === "stale").length;
  const added = fresh.files.filter((f) => f.state === "new").length;
  const behind = stale + added;

  if (fresh.indexing) {
    return {
      state: "updating",
      headline: "Catching up…",
      sub: `reading your latest changes in ${name}`,
      behindCount: behind,
    };
  }
  if (behind > 0) {
    const noun = behind === 1 ? "file" : "files";
    return {
      state: "stale",
      headline: `Your AI is ${behind} ${noun} behind`,
      sub:
        added > 0 && stale === 0
          ? `it hasn't seen ${added === 1 ? "a new file" : `${added} new files`} yet`
          : "it may answer from old code",
      behindCount: behind,
    };
  }
  return {
    state: "ok",
    headline: "Your AI sees your code — up to date",
    sub: "everything it reads matches your working tree",
    behindCount: 0,
  };
}

/* ----------------------------------------------------------------- */
/* Mirror tree building                                               */
/* ----------------------------------------------------------------- */

export interface TreeNode {
  name: string;
  path: string;
  /** Worst state in the subtree for dirs; the file's own state for leaves. */
  state: TrustState;
  /** The daemon's raw verdict (leaves only) — drives the note wording. */
  raw?: FileFreshness["state"];
  children: TreeNode[];
  isFile: boolean;
}

const SEVERITY: Record<string, number> = {
  ok: 0,
  updating: 1,
  hidden: 1,
  stale: 2,
};

function toTrust(state: FileFreshness["state"]): TrustState {
  // `new` and `missing` both read as "behind your changes" to the user.
  return state === "current" ? "ok" : "stale";
}

function worst(a: TrustState, b: TrustState): TrustState {
  return (SEVERITY[b] ?? 0) > (SEVERITY[a] ?? 0) ? b : a;
}

/** Build a nested tree with worst-state-wins directory roll-ups. */
export function buildTree(files: FileFreshness[]): TreeNode[] {
  const root: TreeNode = { name: "", path: "", state: "ok", children: [], isFile: false };
  for (const f of files) {
    const parts = f.path.split("/");
    let node = root;
    let acc = "";
    for (let i = 0; i < parts.length; i++) {
      acc = acc ? `${acc}/${parts[i]}` : parts[i];
      const isLeaf = i === parts.length - 1;
      let child = node.children.find((c) => c.name === parts[i]);
      if (!child) {
        child = { name: parts[i], path: acc, state: "ok", children: [], isFile: isLeaf };
        node.children.push(child);
      }
      node = child;
    }
    node.state = toTrust(f.state);
    node.raw = f.state;
  }
  rollUp(root);
  sortTree(root);
  return root.children;
}

function rollUp(node: TreeNode): TrustState {
  if (node.isFile) return node.state;
  let acc: TrustState = "ok";
  for (const c of node.children) acc = worst(acc, rollUp(c));
  node.state = acc;
  return acc;
}

function sortTree(node: TreeNode) {
  // Dirs before files, then alphabetical — a file browser's expected order.
  node.children.sort((a, b) =>
    a.isFile === b.isFile ? a.name.localeCompare(b.name) : a.isFile ? 1 : -1,
  );
  node.children.forEach(sortTree);
}

/** The plain-words note for a leaf, honest per raw verdict (DESIGN §2.5). */
export function leafNote(raw: FileFreshness["state"] | undefined): string | undefined {
  switch (raw) {
    case "stale":
      return "your AI sees an older version";
    case "new":
      return "your AI hasn't seen this yet";
    case "missing":
      return "deleted — your AI still remembers it";
    default:
      return undefined;
  }
}

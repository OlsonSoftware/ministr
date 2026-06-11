import { useState } from "react";
import { TrustPanel } from "./components/home/TrustPanel";
import { ProjectMirror } from "./components/mirror/ProjectMirror";
import { ProofFeed } from "./components/feed/ProofFeed";
import { ConnectFlow } from "./components/connect/ConnectFlow";
import type { CorpusInfo } from "./lib/ipc";

type View =
  | { kind: "home" }
  | { kind: "connect" }
  | { kind: "mirror"; corpus: CorpusInfo }
  | { kind: "feed"; corpus: CorpusInfo };

/**
 * App shell — hub-and-spoke, two levels (UX-BLUEPRINT navigation):
 * Home (Trust Panel) → Project Mirror → Proof Feed, plus the first-run
 * Connect flow (also reachable from Home's add-project).
 */
export default function App() {
  const [view, setView] = useState<View>({ kind: "home" });

  return (
    <div className="min-h-screen bg-bg text-ink">
      {view.kind === "connect" ? (
        <ConnectFlow onDone={() => setView({ kind: "home" })} />
      ) : view.kind === "home" ? (
        <TrustPanel
          onOpenProject={(corpus) => setView({ kind: "mirror", corpus })}
          onAddProject={() => setView({ kind: "connect" })}
        />
      ) : view.kind === "mirror" ? (
        <ProjectMirror
          corpus={view.corpus}
          onBack={() => setView({ kind: "home" })}
          onOpenFeed={() => setView({ kind: "feed", corpus: view.corpus })}
        />
      ) : (
        <ProofFeed
          corpus={view.corpus}
          onBack={() => setView({ kind: "mirror", corpus: view.corpus })}
        />
      )}
    </div>
  );
}

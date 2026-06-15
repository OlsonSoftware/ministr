import { useEffect, useState } from "react";
import { TrustPanel } from "./components/home/TrustPanel";
import { ProjectManage } from "./components/manager/ProjectManage";
import { ConnectFlow } from "./components/connect/ConnectFlow";
import { Screen } from "./components/ui/Screen";
import { Brand } from "./components/ui/Brand";
import { Beat } from "./components/ui/Beat";
import { listCorpora, type CorpusInfo } from "./lib/ipc";

type View =
  | { kind: "boot" }
  | { kind: "home" }
  | { kind: "connect"; firstRun?: boolean }
  | { kind: "mirror"; corpus: CorpusInfo };

/**
 * App shell (GUI v6 — index manager): Home (the index list) → ProjectManage
 * (per-index management), plus the first-run Connect flow (also reachable
 * from Home's add-project). The `kind: "mirror"` view now renders the
 * management panel (the name is kept to avoid churn).
 */
export default function App() {
  const [view, setView] = useState<View>({ kind: "boot" });

  // First-run gate (gui-ux-first-run-onboarding): a user who never
  // registered a project must NOT land on an empty Home. Ask the daemon
  // once on launch — no corpora → the Connect welcome; otherwise Home.
  // If the daemon is unreachable, fall through to Home, which renders its
  // own honest "ministr isn't running" state rather than a misleading
  // welcome.
  useEffect(() => {
    let cancelled = false;
    void listCorpora()
      .then((corpora) => {
        if (cancelled) return;
        setView(
          corpora.length === 0
            ? { kind: "connect", firstRun: true }
            : { kind: "home" },
        );
      })
      .catch(() => {
        if (!cancelled) setView({ kind: "home" });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Escape walks back one level (gui-rw-keyboard-flow). Screens that
  // consume Escape themselves (the Mirror's drill-in) preventDefault
  // first, so this only fires for top-level navigation.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      setView((v) =>
        v.kind === "mirror" || v.kind === "connect" ? { kind: "home" } : v,
      );
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="min-h-screen bg-bg text-ink">
      {view.kind === "boot" ? (
        <Screen align="center" gap="lg" footer={null}>
          <div className="flex flex-col items-center gap-6">
            <Brand />
            <Beat sentence="starting ministr…" />
          </div>
        </Screen>
      ) : view.kind === "connect" ? (
        <ConnectFlow
          firstRun={view.firstRun}
          onDone={() => setView({ kind: "home" })}
        />
      ) : view.kind === "home" ? (
        <TrustPanel
          onOpenProject={(corpus) => setView({ kind: "mirror", corpus })}
          onAddProject={() => setView({ kind: "connect" })}
        />
      ) : (
        <ProjectManage
          corpus={view.corpus}
          onBack={() => setView({ kind: "home" })}
        />
      )}
    </div>
  );
}

import { useState } from "react";
import { TrustPanel } from "./components/home/TrustPanel";
import { ProjectMirror } from "./components/mirror/ProjectMirror";
import type { CorpusInfo } from "./lib/ipc";

/**
 * App shell — hub-and-spoke, two levels (UX-BLUEPRINT navigation):
 * Home (Trust Panel) ⇄ Project Mirror. No tabs, no router.
 */
export default function App() {
  const [open, setOpen] = useState<CorpusInfo | null>(null);

  return (
    <div className="min-h-screen bg-bg text-ink">
      {open ? (
        <ProjectMirror corpus={open} onBack={() => setOpen(null)} />
      ) : (
        <TrustPanel onOpenProject={setOpen} />
      )}
    </div>
  );
}

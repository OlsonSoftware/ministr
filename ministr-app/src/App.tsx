import { Keel } from "./components/Keel";

/**
 * App shell — clean slate (gui-rw-archive-scaffold).
 *
 * The previous app (facet workspace) is archived verbatim on the
 * `archive/app-v1` branch. This shell is intentionally bare: the rewrite
 * lands screen by screen per ministr-app/UX-BLUEPRINT.md v4
 * (Home Trust Panel → Project Mirror → Proof Feed → connect flow).
 */
export default function App() {
  return (
    <main className="min-h-screen bg-bg text-text grid place-items-center">
      <Keel
        title="ministr"
        line="rebuilding from the keel — UX-BLUEPRINT v4"
      />
    </main>
  );
}

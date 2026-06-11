import { BetaClient } from "./beta-client";

export default function BetaPage() {
  return (
    <>
      <section className="v2-section" style={{ paddingBottom: 0 }}>
        <p className="v2-meta" style={{ marginBottom: 16 }}>
          Closed beta
        </p>
        <h1 className="v2-h2">Early access to ministr</h1>
        <p className="v2-sub">
          ministr is in a closed beta. Request access below; once
          you&apos;re approved, sign in with GitHub and the download
          unlocks on this page.
        </p>
      </section>

      <hr className="v2-rule" />

      <section className="v2-section">
        <div style={{ maxWidth: "28rem" }}>
          <BetaClient />
        </div>
      </section>
    </>
  );
}

export const metadata = {
  title: "Closed beta",
  description:
    "Request access to the ministr closed beta and download early builds.",
};

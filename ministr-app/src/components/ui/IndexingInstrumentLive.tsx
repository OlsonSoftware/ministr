import { IndexingInstrument } from "./IndexingInstrument";
import { useIngestionProgress } from "../../lib/useIngestionProgress";

/**
 * Live Storybook host for the Indexing Instrument: renders the first
 * polled corpus through the real hook → derive path. A normal module
 * (not the .stories.tsx) because hook-calling components defined inside
 * story files break under the React Compiler's useMemoCache pass.
 */
export function IndexingInstrumentLive({
  intervalMs = 300,
  variant = "full",
}: {
  intervalMs?: number;
  variant?: "compact" | "full";
}) {
  const { progress } = useIngestionProgress(intervalMs);
  const first = [...progress.values()][0];
  return first ? (
    <IndexingInstrument progress={first} variant={variant} />
  ) : (
    <p className="text-sm text-dim">waiting for first poll…</p>
  );
}

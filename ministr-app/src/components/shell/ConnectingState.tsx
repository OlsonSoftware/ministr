import { AlertTriangle } from "@/components/ui/icons";
import { Logo } from "../ui/logo";
import { StatusDot } from "../ui/status-dot";
import { cn } from "../../lib/utils";

interface ConnectingStateProps {
  /** Daemon error message, or null while a connection is still being attempted. */
  error: string | null;
}

/**
 * The boot screen — the first thing shown on every cold launch, until the
 * daemon status resolves (App renders it whenever `status` is null).
 *
 * A command-deck "starting up" hero rather than a bare spinner: a glowing
 * object medallion + the ministr wordmark + a live status line. When the
 * daemon is unreachable the medallion goes quiet (danger) and the reason is
 * surfaced as an inline alert. Tone rides the medallion glow + the status dot;
 * the wordmark, status label and error text stay full-contrast for AA.
 */
export function ConnectingState({ error }: ConnectingStateProps) {
  const failed = error != null;
  return (
    <div className="flex h-full flex-col items-center justify-center gap-7 px-6 text-center">
      <div className="flex flex-col items-center gap-5">
        <span
          aria-hidden
          className={cn(
            "grid h-16 w-16 shrink-0 place-items-center rounded-2xl border bg-surface-overlay",
            failed
              ? "border-danger/50 text-danger"
              : "border-accent/50 text-accent shadow-[var(--glow-soft)]",
          )}
        >
          {/* The real brand mark — full-colour while connecting; mono so it
              tones with the danger medallion when the daemon is unreachable. */}
          <Logo className="h-7 w-7" gradient={!failed} />
        </span>

        <div className="flex flex-col items-center gap-2.5">
          <div className="flex items-center gap-1.5">
            <span className="font-mono text-2xl font-semibold tracking-[-0.01em] text-text">
              ministr
            </span>
            <span aria-hidden className="h-2 w-2 rounded-full bg-accent" />
          </div>
          <div className="flex items-center gap-2">
            <StatusDot
              tone={failed ? "danger" : "accent"}
              pulse={failed ? "off" : "live"}
              size="md"
            />
            <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">
              {failed ? "Connection failed" : "Connecting to the daemon"}
            </span>
          </div>
        </div>
      </div>

      {error && (
        <div
          role="alert"
          className="flex max-w-md items-start gap-2.5 rounded-lg border border-danger/40 bg-danger/10 px-3.5 py-2.5 text-left"
        >
          <AlertTriangle
            aria-hidden
            className="mt-0.5 h-4 w-4 shrink-0 text-danger"
            strokeWidth={2}
          />
          <p className="font-sans text-sm leading-snug text-text">{error}</p>
        </div>
      )}
    </div>
  );
}

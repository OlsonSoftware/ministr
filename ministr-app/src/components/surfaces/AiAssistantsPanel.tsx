import { Sparkles } from "lucide-react";
import { cn } from "../../lib/utils";

/**
 * AI Assistants panel — the in-app MCP setup wizard.
 *
 * M1 STUB: this is a placeholder card explaining what the panel will do.
 * M3 will:
 *   1. Add backend Tauri commands `mcp_detect_clients`, `mcp_write_config`,
 *      `mcp_test_connection` (see plan M3).
 *   2. Refactor `write_mcp_configs` in ministr-core/src/init.rs to be
 *      per-client + add Codex.
 *   3. Render one row per supported client (Claude Code / Cursor /
 *      VS Code Copilot / Codex) with detection state + Connect button +
 *      live connection test.
 */
export function AiAssistantsPanel() {
  return (
    <div className="space-y-4">
      <header className="space-y-1">
        <h2 className="font-mono text-sm font-bold uppercase tracking-[0.05em] text-text">
          AI assistants
        </h2>
        <p className="font-serif text-sm text-text-muted">
          Connect ministr to the AI tools you use — one click each.
        </p>
      </header>

      <div
        className={cn(
          "border-2 border-border bg-surface p-4 flex items-start gap-3",
        )}
      >
        <Sparkles className="h-4 w-4 mt-0.5 shrink-0 text-accent" strokeWidth={2.5} />
        <div className="space-y-2 min-w-0">
          <div className="font-mono text-xs font-semibold uppercase tracking-[0.05em] text-text">
            Wizard coming next
          </div>
          <p className="font-serif text-sm text-text-muted">
            The wizard will detect Claude Code, Cursor, VS Code Copilot, and
            Codex on your system, write the right config file for each, and
            run a live connection test so you know it works before you switch
            apps. For now, see the README for the manual command.
          </p>
        </div>
      </div>
    </div>
  );
}

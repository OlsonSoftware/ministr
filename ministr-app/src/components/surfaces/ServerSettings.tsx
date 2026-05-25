/**
 * ServerSettings — read-only server vitals + diagnostics.
 *
 * The background ministr process's version, embedding model, memory, and
 * file paths, plus the collapsible log viewer and context simulator.
 * Diagnostics live here (rather than the Developer tab) so the log path
 * and the live log are reachable from one place; the Developer tab's
 * Logs sub-view is the same component for the power-user workflow.
 */
import { useEffect, useRef, useState } from "react";
import { ScrollText, Terminal } from "lucide-react";

import type { DaemonStatus } from "../../lib/types";
import { ContentTray } from "../ui/content-tray";
import { LogViewer } from "../LogViewer";
import { ContextSimulator } from "../ContextSimulator";
import { SettingsSection, MetaRow, DiagnosticSection } from "./settings-primitives";

/** Detail payload for the `ministr-settings-scroll` window event. */
export type SettingsScrollTarget = "logs" | "simulator";

const DATA_DIR = "~/.ministr/";

interface Props {
  status: DaemonStatus;
}

export function ServerSettings({ status }: Props) {
  const [logsExpanded, setLogsExpanded] = useState(false);
  const [simulatorExpanded, setSimulatorExpanded] = useState(false);
  const logsRef = useRef<HTMLDivElement>(null);
  const simulatorRef = useRef<HTMLDivElement>(null);

  // Other parts of the app (status-dot "open log file" fallback, Cmd+K)
  // dispatch a window event to jump straight to a diagnostic section.
  useEffect(() => {
    function onScroll(e: Event) {
      const detail = (e as CustomEvent).detail as
        | SettingsScrollTarget
        | undefined;
      if (detail === "logs") {
        setLogsExpanded(true);
        requestAnimationFrame(() => {
          logsRef.current?.scrollIntoView({
            behavior: "smooth",
            block: "start",
          });
        });
      } else if (detail === "simulator") {
        setSimulatorExpanded(true);
        requestAnimationFrame(() => {
          simulatorRef.current?.scrollIntoView({
            behavior: "smooth",
            block: "start",
          });
        });
      }
    }
    window.addEventListener("ministr-settings-scroll", onScroll);
    return () => {
      window.removeEventListener("ministr-settings-scroll", onScroll);
    };
  }, []);

  return (
    <div>
      <SettingsSection title="Server" description="Read-only" />
      <ContentTray>
        <MetaRow label="VERSION" value={`v${status.version}`} />
        <MetaRow label="EMBEDDING MODEL" value={status.model} />
        <MetaRow
          label="MEMORY"
          value={`${status.memory_mb.toFixed(0)} MB RSS`}
        />
        <MetaRow label="DATA DIR" value={DATA_DIR} />
        {status.log_path && (
          <MetaRow label="LOG FILE" value={status.log_path} truncate />
        )}
      </ContentTray>

      <SettingsSection title="Diagnostics" />
      <ContentTray className="overflow-hidden !p-0">
      <div ref={logsRef}>
        <DiagnosticSection
          icon={ScrollText}
          label="Server log"
          hint="Recent log lines from the running ministr server"
          expanded={logsExpanded}
          onToggle={() => setLogsExpanded((v) => !v)}
          isLast={false}
        >
          <div className="max-h-[420px] overflow-hidden">
            <LogViewer />
          </div>
        </DiagnosticSection>
      </div>
      <div ref={simulatorRef}>
        <DiagnosticSection
          icon={Terminal}
          label="Context simulator"
          hint="Replay a project query against the current session model"
          expanded={simulatorExpanded}
          onToggle={() => setSimulatorExpanded((v) => !v)}
          isLast={true}
        >
          <ContextSimulator />
        </DiagnosticSection>
      </div>
      </ContentTray>
    </div>
  );
}

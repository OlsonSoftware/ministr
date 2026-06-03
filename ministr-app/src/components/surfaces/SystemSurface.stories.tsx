import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CorpusInfo, DaemonStatus } from "../../lib/types";
import type { McpClientView } from "../../hooks/useMcpClients";
import { SystemSurface } from "./SystemSurface";

/**
 * SystemSurface — the Account "System" tab (aaa-settings): Settings dissolved
 * into one thin, diagnostics-led global surface. States exercise the fleet
 * health (healthy / indexing / error) and the AI integration card states.
 * Framed the size of the Account overlay body.
 */

function mkCorpus(over: Partial<CorpusInfo> & { id: string }): CorpusInfo {
  return {
    paths: [`/Users/alrik/Code/${over.id}`],
    display_name: over.id,
    status: { state: "idle" },
    files_indexed: 1200,
    sections_count: 8000,
    embeddings_count: 30000,
    active_sessions: 0,
    symbols_count: 9000,
    ...over,
  };
}

const CORPORA: CorpusInfo[] = [
  mkCorpus({ id: "ministr", files_indexed: 4821, embeddings_count: 38104, active_sessions: 2 }),
  mkCorpus({ id: "ministr-app", files_indexed: 1240, embeddings_count: 6203 }),
  mkCorpus({ id: "design-system", files_indexed: 612, embeddings_count: 4188, active_sessions: 1 }),
];

const STATUS: DaemonStatus = {
  version: "0.2.1",
  uptime_secs: 198_540,
  memory_mb: 412,
  model: "jina-code-v2",
  model_dimension: 768,
  corpora: CORPORA,
  total_sessions: 3,
  autostart_enabled: true,
  log_path: "/Users/alrik/.ministr/ministr.log",
};

function mkClient(
  id: string,
  display_name: string,
  state: McpClientView["state"],
  ok = false,
): McpClientView {
  const installed = state !== "not_installed";
  const configured = state === "configured" || state === "connected";
  return {
    info: {
      id,
      display_name,
      installed,
      configured,
      config_path: `~/.config/${id}/mcp.json`,
    },
    state,
    lastTest:
      state === "connected"
        ? { ok, message: "ministr responded · 5 tools", raw_output_truncated: null, manual_verify_needed: false }
        : state === "configured"
          ? { ok: false, message: "Config written — restart the editor to verify", raw_output_truncated: null, manual_verify_needed: true }
          : null,
    lastTestAt: state === "connected" || state === "configured" ? 1 : null,
  };
}

const INTEGRATIONS: McpClientView[] = [
  mkClient("claude-code", "Claude Code", "connected", true),
  mkClient("cursor", "Cursor", "configured"),
  mkClient("copilot", "VS Code Copilot", "not_configured"),
  mkClient("codex", "Codex CLI", "not_installed"),
];

const noop = () => {};

const meta = {
  title: "Surfaces/SystemSurface",
  component: SystemSurface,
  parameters: { layout: "fullscreen" },
  args: {
    theme: "dark",
    density: "comfortable",
    onThemeChange: noop,
    onDensityChange: noop,
    onToggleAutostart: noop,
    onConnectIntegration: noop,
    onTestIntegration: noop,
    onOpenIntegrationFile: noop,
    onRefreshIntegrations: noop,
    onOpenDataFolder: noop,
    onOpenLogs: noop,
    onRerunOnboarding: noop,
    onResetPreferences: noop,
    onClearCache: noop,
    onCheckUpdates: noop,
    projectRoot: "/Users/alrik/Code/ministr",
  },
  decorators: [
    (Story) => (
      <div className="h-[680px] w-full bg-bg p-4 sm:p-8 grid place-items-center">
        <div className="flex h-full w-full max-w-4xl flex-col overflow-hidden rounded-xl border border-border bg-surface shadow-2xl">
          <div className="flex h-12 shrink-0 items-center border-b border-border px-5 font-sans text-sm font-semibold text-text">
            Account
          </div>
          <div className="min-h-0 flex-1 overflow-hidden">
            <Story />
          </div>
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof SystemSurface>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Healthy fleet + mixed AI integration states — the full surface. */
export const Default: Story = {
  args: { status: STATUS, integrations: INTEGRATIONS },
};

/** A project mid-index — fleet health reads warning. */
export const Indexing: Story = {
  args: {
    status: {
      ...STATUS,
      corpora: [
        mkCorpus({
          id: "ministr-app",
          status: { state: "indexing", files_done: 740, files_total: 1284 },
        }),
        ...CORPORA,
      ],
    },
    integrations: INTEGRATIONS,
  },
};

/** An index in an error state — fleet health reads danger. */
export const Error: Story = {
  args: {
    status: {
      ...STATUS,
      corpora: [
        mkCorpus({ id: "broken", status: { state: "error", message: "embed failed" } }),
        ...CORPORA,
      ],
    },
    integrations: INTEGRATIONS,
  },
};

/** No project selected — integrations point back to the spine. */
export const NoProject: Story = {
  args: { status: STATUS, integrations: [], projectRoot: null },
};

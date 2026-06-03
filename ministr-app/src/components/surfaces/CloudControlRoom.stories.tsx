import type { Meta, StoryObj } from "@storybook/react-vite";
import type {
  CloudCorpusInfo,
  CloudStatus,
  CloudUsage,
} from "../../lib/cloudClient";
import { CloudControlRoom } from "./CloudControlRoom";

/**
 * CloudControlRoom — the Account area's dashboard-first cloud control room
 * (aaa-cloud). States follow the live connection: anon (connect CTA) →
 * connected dashboard (living status + usage economics + corpora-as-assets +
 * automation) → connected-but-empty. Rendered inside a panel frame the size of
 * the real Account overlay body.
 */

const ENDPOINT = "https://mcp.ministr.ai";

function isoDaysAgo(n: number): string {
  // Deterministic-ish day strings; the component only sorts + counts them.
  const d = new Date(Date.UTC(2026, 5, 3) - n * 86_400_000);
  return d.toISOString().slice(0, 10);
}

const CONNECTED: CloudStatus = {
  configured: true,
  authenticated: true,
  endpoint: ENDPOINT,
  last_health_ok: true,
  last_health_latency_ms: 38,
  last_health_message: "ok",
};

const ANON: CloudStatus = {
  configured: true,
  authenticated: false,
  endpoint: ENDPOINT,
  last_health_ok: null,
  last_health_latency_ms: null,
  last_health_message: null,
};

const USAGE: CloudUsage = {
  tenant_id: "demo-tenant",
  plan: "team",
  rollups: [13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1].flatMap((n) => [
    { day: isoDaysAgo(n), kind: "survey", total: 40 + ((n * 17) % 60) },
    { day: isoDaysAgo(n), kind: "ask", total: 8 + ((n * 7) % 20) },
    { day: isoDaysAgo(n), kind: "extract", total: 3 + ((n * 5) % 11) },
  ]),
  today_partial: [
    { kind: "survey", total: 23 },
    { kind: "ask", total: 5 },
  ],
};

const CORPORA: CloudCorpusInfo[] = [
  {
    corpus_id: "acme-platform",
    paths: ["github.com/acme/platform"],
    display_name: "acme/platform",
    indexing_status: "ready",
    total_files: 4821,
    total_chunks: 38104,
    active_sessions: 2,
  },
  {
    corpus_id: "acme-web",
    paths: ["github.com/acme/web"],
    display_name: "acme/web",
    indexing_status: "indexing",
    total_files: 1240,
    total_chunks: 6203,
    active_sessions: 0,
  },
  {
    corpus_id: "design-system",
    paths: ["github.com/acme/design-system"],
    display_name: "acme/design-system",
    indexing_status: "ready",
    total_files: 612,
    total_chunks: 4188,
    active_sessions: 1,
  },
];

const noop = () => {};

const meta = {
  title: "Surfaces/CloudControlRoom",
  component: CloudControlRoom,
  parameters: { layout: "fullscreen" },
  args: {
    onConnect: noop,
    onDisconnect: noop,
    onManageBilling: noop,
    onRefresh: noop,
  },
  decorators: [
    (Story) => (
      // Frame the size of the Account overlay body.
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
} satisfies Meta<typeof CloudControlRoom>;

export default meta;
type Story = StoryObj<typeof meta>;

/** First status load still in flight. */
export const Loading: Story = {
  args: {
    connection: null,
    usage: null,
    corpora: [],
    apiKeyCount: 0,
    webhookCount: 0,
    loading: true,
  },
};

/** Not signed in — one clear call to value. */
export const Anonymous: Story = {
  args: {
    connection: ANON,
    usage: null,
    corpora: [],
    apiKeyCount: 0,
    webhookCount: 0,
  },
};

/** Connected with real usage + corpora-as-assets — the full dashboard. */
export const Connected: Story = {
  args: {
    connection: CONNECTED,
    usage: USAGE,
    corpora: CORPORA,
    apiKeyCount: 2,
    webhookCount: 1,
  },
};

/** Connected but nothing synced yet — honest empties, not a blank. */
export const ConnectedEmpty: Story = {
  args: {
    connection: CONNECTED,
    usage: { tenant_id: "demo", plan: "pro", rollups: [], today_partial: [] },
    corpora: [],
    apiKeyCount: 0,
    webhookCount: 0,
  },
};

/** Health probe failing — degraded connection reads danger, not silent. */
export const Degraded: Story = {
  args: {
    connection: {
      ...CONNECTED,
      last_health_ok: false,
      last_health_latency_ms: null,
      last_health_message: "503 upstream",
    },
    usage: USAGE,
    corpora: CORPORA,
    apiKeyCount: 2,
    webhookCount: 1,
  },
};

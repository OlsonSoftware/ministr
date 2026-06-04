import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import type { ActivityEvent } from "../../../lib/types";
import { SessionCodeTouchedSection } from "./SessionCodeTouchedSection";

/**
 * §1 "Code touched" — derived purely from the session's activity stream
 * (summarizeCodeTouched). Each touched SYMBOL chip is a cross-facet deep-link:
 * clicking it jumps into the Explore facet at that symbol
 * (aaa-explore-session-codetouched). File rows filter the activity timeline.
 */

const ev = (over: Partial<ActivityEvent>): ActivityEvent => ({
  timestamp_ms: Date.now(),
  tool: "ministr_read",
  corpus_id: "ministr",
  summary: "",
  cache_hit: false,
  duration_ms: 10,
  ...over,
});

// Summaries follow the daemon's tolerant format; `/./` marks the repo-root
// boundary so paths relativize to `src/…`.
const EVENTS: ActivityEvent[] = [
  ev({
    tool: "ministr_definition",
    summary: "verify_token — /Users/a/Code/ministr/./src/auth/middleware.rs",
  }),
  ev({
    tool: "ministr_references",
    summary:
      "create_session — /Users/a/Code/ministr/./src/session/registry.rs (12)",
  }),
  ev({
    tool: "ministr_definition",
    summary: "AppState — /Users/a/Code/ministr/./src/daemon/state.rs",
  }),
  ev({ tool: "ministr_read", summary: "src/auth/middleware.rs#verify_token" }),
  ev({ tool: "ministr_extract", summary: 'src/session/registry.rs · "budget" (3)' }),
];

function Frame({ children }: { children: ReactNode }) {
  return <div className="w-[420px] bg-bg p-4">{children}</div>;
}

const meta = {
  title: "Entity/SessionCodeTouchedSection",
  parameters: { layout: "centered" },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/** With `onOpenSymbol` wired — the symbol chips are interactive deep-links. */
export const Interactive: Story = {
  render: () => (
    <Frame>
      <SessionCodeTouchedSection
        chapter={1}
        events={EVENTS}
        loading={false}
        onFilterFile={() => {}}
        onOpenSymbol={() => {}}
      />
    </Frame>
  ),
};

/** Storied in isolation without a workspace — chips render inert (no jump). */
export const Static: Story = {
  render: () => (
    <Frame>
      <SessionCodeTouchedSection chapter={1} events={EVENTS} loading={false} />
    </Frame>
  ),
};

/** No code-navigation activity yet. */
export const Empty: Story = {
  render: () => (
    <Frame>
      <SessionCodeTouchedSection chapter={1} events={[]} loading={false} />
    </Frame>
  ),
};

/** First poll in flight. */
export const Loading: Story = {
  render: () => (
    <Frame>
      <SessionCodeTouchedSection chapter={1} events={[]} loading />
    </Frame>
  ),
};

import type { Meta, StoryObj } from "@storybook/react-vite";
import { Database, FileText, Hash, Layers } from "lucide-react";
import { Button } from "./button";
import { Badge } from "./badge";
import { StatusDot } from "./status-dot";
import { Progress } from "./progress";
import { BudgetBar } from "./budget-bar";
import { BudgetRing } from "./budget-ring";
import { TokenEconomicsBar } from "./token-economics-bar";
import { Sparkline } from "./sparkline";
import { MetricTile } from "./metric-tile";
import { FilterPill } from "./filter-pill";
import { Disclosure } from "./disclosure";
import { Card } from "./card";
import { LabeledCard } from "./labeled-card";
import { LabeledRow } from "./labeled-row";
import { ContentTray } from "./content-tray";
import { EmptyState } from "./empty-state";
import { ErrorCallout } from "./error-callout";
import { Toggle } from "./toggle";
import { H3 } from "./heading";

/**
 * Kitchen-sink overview of the ui/ primitive library — the at-a-glance
 * regression surface for the v4 design contract. Review in BOTH light and
 * dark (toolbar theme toggle) when polishing any primitive.
 */
const meta = { title: "UI/Overview" } satisfies Meta;
export default meta;
type Story = StoryObj<typeof meta>;

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  // Inline gap: Tailwind doesn't emit story-only gap-* utilities in the
  // Storybook build, so the section rhythm is set with a guaranteed style.
  return (
    <section className="flex flex-col" style={{ gap: "0.75rem" }}>
      <H3>{label}</H3>
      {children}
    </section>
  );
}

const SERIES = [12, 18, 9, 22, 30, 24, 33, 28, 41, 38, 52, 47];

export const Library: Story = {
  render: () => (
    <div
      className="mx-auto flex max-w-5xl flex-col p-2"
      style={{ gap: "3rem" }}
    >
      <Section label="Buttons">
        <div className="flex flex-wrap items-center gap-3">
          <Button>Run query</Button>
          <Button variant="outline">Outline</Button>
          <Button variant="ghost">Ghost</Button>
          <Button variant="subtle">Subtle</Button>
          <Button variant="danger">Delete</Button>
          <Button disabled>Disabled</Button>
        </div>
      </Section>

      <Section label="Badges · StatusDots · Toggle">
        <div className="flex flex-wrap items-center gap-3">
          <Badge variant="success" dot>ready</Badge>
          <Badge variant="warning" dot>warming</Badge>
          <Badge variant="danger" dot>error</Badge>
          <Badge variant="muted">muted</Badge>
          <span className="inline-flex items-center gap-2">
            <StatusDot tone="success" /> <StatusDot tone="accent" pulse="live" size="md" />
            <StatusDot tone="danger" />
          </span>
          <Toggle enabled onToggle={() => {}} ariaLabel="on" />
          <Toggle enabled={false} onToggle={() => {}} ariaLabel="off" />
        </div>
      </Section>

      <Section label="Filter pills">
        <div className="flex flex-wrap items-center gap-2">
          <FilterPill label="All" count={312} active onClick={() => {}} />
          <FilterPill label="Rust" count={184} active={false} onClick={() => {}} />
          <FilterPill label="TypeScript" count={96} active={false} onClick={() => {}} />
        </div>
      </Section>

      <Section label="Meters · Charts">
        <div className="flex flex-wrap items-center gap-8">
          <div className="flex min-w-[280px] flex-1 flex-col gap-3">
            <Progress value={62} tone="accent" glow />
            <BudgetBar utilization={0.62} size="hero" showValue />
            <TokenEconomicsBar deliveredTokens={184_000} savedTokens={96_000} liveTokens={42_000} />
            <div className="w-60">
              <Sparkline data={SERIES} smooth height={48} ariaLabel="tokens over time" />
            </div>
          </div>
          <BudgetRing utilization={0.62} warm={0.2} pressure="medium">
            <span className="font-mono text-lg font-semibold tabular-nums text-text">62%</span>
            <span className="font-mono text-mono-mini uppercase tracking-[0.08em] text-text-dim">budget</span>
          </BudgetRing>
        </div>
      </Section>

      <Section label="Metric tiles">
        <div className="grid grid-cols-3 gap-3">
          <MetricTile icon={FileText} label="Files" value="1,204" />
          <MetricTile icon={Layers} label="Sections" value="12,840" tone="accent" />
          <MetricTile icon={Hash} label="Symbols" value="41,902" />
        </div>
      </Section>

      <Section label="Cards · Trays · Disclosure">
        <div className="grid grid-cols-2 gap-4">
          <Card hover="lift" className="text-sm text-text">Tier-1 card · hover to lift</Card>
          <LabeledCard title="Corpus config" icon={Database} iconTone="accent">
            <ContentTray compact>
              <LabeledRow label="model" value="jina-code-v2" mono bordered />
              <LabeledRow label="dimension" value="768" mono />
            </ContentTray>
          </LabeledCard>
          <div className="col-span-2">
            <Disclosure title="Retrieval settings" chapter={3} meta="4 keys" defaultOpen>
              <div className="px-4 py-3 text-sm text-text-muted">
                Hybrid retrieval, reranker depth, and Matryoshka dimension.
              </div>
            </Disclosure>
          </div>
        </div>
      </Section>

      <Section label="Empty · Error states">
        <div className="grid grid-cols-2 gap-4">
          <EmptyState icon={FileText} title="No projects yet" hint="Index a repo to begin." />
          <ErrorCallout title="Indexing failed" message="daemon error: corpus not found" />
        </div>
      </Section>
    </div>
  ),
};

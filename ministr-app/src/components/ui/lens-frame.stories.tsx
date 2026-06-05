import type { Meta, StoryObj } from "@storybook/react-vite";
import { Cable, Sparkles, Stethoscope } from "@/components/ui/icons";
import { LensHeader, LensLoading, LensEmpty, LensRerunButton } from "./lens-frame";

/**
 * lens-frame — the shared lens-chrome grammar (aaa-explore-integration-cohesion).
 * One header / loading / empty template so the six Explore lenses read as one
 * system. Each lens keeps its own rich glance + filter content.
 */
const meta = {
  title: "UI/LensFrame",
  parameters: { layout: "fullscreen" },
  decorators: [
    (Story) => (
      <div className="@container/page h-[520px] w-full bg-bg">
        <div className="mx-auto flex h-full max-w-2xl flex-col border-x border-border bg-surface">
          <Story />
        </div>
      </div>
    ),
  ],
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/** The header — toned title, an inline glance stat line, filter chips, hint. */
export const Header: Story = {
  render: () => (
    <LensHeader
      icon={Cable}
      title="Cross-language bridges"
      tone="accent"
      glance={
        <>
          <span className="tabular-nums font-semibold text-text">42</span> seams ·{" "}
          <span className="tabular-nums font-semibold text-text">5</span> mechanisms ·{" "}
          <span className="tabular-nums font-semibold text-text">3</span> languages
        </>
      }
      hint="Where one language calls into another — Tauri, PyO3, NAPI, wasm-bindgen, HTTP-route, FFI."
      onRefresh={() => {}}
    >
      <div className="flex flex-wrap gap-1.5">
        {["All", "Tauri command", "PyO3", "HTTP route"].map((l, i) => (
          <span
            key={l}
            className={
              "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 font-mono text-mono-mini uppercase tracking-[0.06em] " +
              (i === 0
                ? "border-accent bg-surface-overlay text-text"
                : "border-border-soft bg-surface text-text-muted")
            }
          >
            <span className="font-semibold">{l}</span>
            <span className="tabular-nums">{[42, 18, 12, 8][i]}</span>
          </span>
        ))}
      </div>
    </LensHeader>
  ),
};

/** A severity-toned header (Diagnostics goes danger when there are errors). */
export const HeaderDanger: Story = {
  render: () => (
    <LensHeader
      icon={Stethoscope}
      title="Diagnostics"
      tone="danger"
      glance={
        <>
          <span className="tabular-nums font-semibold text-danger">7</span> errors ·{" "}
          <span className="tabular-nums font-semibold text-warning">12</span> warnings
        </>
      }
      hint="The project's own toolchain (cargo · tsc · eslint · ruff · go vet) normalised to one shape."
    />
  ),
};

/** The shared loading line. */
export const Loading: Story = {
  render: () => <LensLoading label="Tracing the reference graph" />,
};

/** The shared empty / error state (the EmptyState atom, centered). */
export const Empty: Story = {
  render: () => (
    <LensEmpty
      icon={Sparkles}
      accent
      title="No dead code"
      hint="Every indexed symbol is referenced (or looks like an entry point). The reference graph is clean."
    />
  ),
};

/** An empty state with a re-run CTA — these views are snapshots, so the empty
 *  state earns a clear next action (2026). */
export const EmptyWithRerun: Story = {
  render: () => (
    <LensEmpty
      icon={Sparkles}
      accent
      title="No dead code"
      hint="Every indexed symbol is referenced (or looks like an entry point). Re-run after editing to re-check."
      action={<LensRerunButton onRefresh={() => {}} />}
    />
  ),
};

/** The refreshing state — the spinner is shown while a re-run is in flight. */
export const Refreshing: Story = {
  render: () => (
    <LensHeader
      icon={Stethoscope}
      title="Diagnostics"
      tone="success"
      glance={<span className="text-text">Clean · re-running…</span>}
      hint="The toolchain is re-running."
      onRefresh={() => {}}
      refreshing
    />
  ),
};

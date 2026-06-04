import type { Meta, StoryObj } from "@storybook/react-vite";
import { Button } from "./button";
import { cn } from "../../lib/utils";
import { focusRing } from "../../lib/ui-tokens";

/**
 * Forced Colors / Windows High Contrast Mode (WHCM) — the DESIGN.md §9 floor.
 *
 * This story is the visual reference for the forced-colors floor. In a normal
 * theme it shows the same interactive atoms we ship everywhere. Its purpose is
 * to be inspected under a forced-colors emulation (Playwright `forcedColors:
 * "active"`, or the DevTools "Emulate CSS forced-colors" rendering toggle):
 * every custom interactive surface MUST keep a visible boundary + focus cue
 * when the UA strips background-color and box-shadow.
 *
 * The floor lives in `app.css` under `@media (forced-colors: active)` — it adds
 * a system-colour border to custom controls + floating panels and pins the
 * focus ring to `Highlight`. No `forced-color-adjust: none` is used: the user's
 * chosen palette always wins.
 */
const meta = {
  title: "A11y/Forced Colors (WHCM)",
  parameters: {
    layout: "padded",
  },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/** A custom interactive surface — the pattern that loses its box in WHCM. */
function ClickableRow({ label, detail }: { label: string; detail: string }) {
  return (
    <div
      role="button"
      tabIndex={0}
      className={cn(
        // boundary drawn by bg + shadow alone — the WHCM-vulnerable pattern
        "flex items-center justify-between gap-4 rounded-md bg-surface-overlay px-3 py-2",
        "cursor-pointer shadow-sm hover:shadow-md transition-shadow",
        focusRing,
      )}
    >
      <span className="text-sm text-text">{label}</span>
      <span className="text-xs text-text-dim">{detail}</span>
    </div>
  );
}

/**
 * The representative interactive atoms, side by side. Inspect under forced
 * colors: every box + the focus ring must remain visible.
 */
export const Showcase: Story = {
  render: () => (
    <div className="flex max-w-xl flex-col gap-6">
      <section className="flex flex-col gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-text-dim">
          Buttons — fill variants draw their box with background only
        </h3>
        <div className="flex flex-wrap items-center gap-3">
          <Button variant="default">Default</Button>
          <Button variant="subtle">Subtle</Button>
          <Button variant="ghost">Ghost</Button>
          <Button variant="outline">Outline</Button>
          <Button variant="danger">Danger</Button>
        </div>
      </section>

      <section className="flex flex-col gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-text-dim">
          Custom interactive rows (role=button) — boundary from bg + shadow
        </h3>
        <div className="flex flex-col gap-2">
          <ClickableRow label="ministr_survey" detail="42 refs" />
          <ClickableRow label="QueryService::compute_diff_impact" detail="src/query.rs" />
        </div>
      </section>

      <section className="flex flex-col gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-text-dim">
          Floating chrome (.glass-panel) — lift is box-shadow only
        </h3>
        <div className="glass-panel max-w-sm p-4">
          <p className="text-sm text-text">Command palette / dialog surface.</p>
          <p className="mt-1 text-xs text-text-dim">
            Keeps a hairline border in forced colors so it stays distinct from
            the canvas.
          </p>
        </div>
      </section>
    </div>
  ),
};

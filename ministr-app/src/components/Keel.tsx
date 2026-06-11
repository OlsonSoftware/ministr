/**
 * Keel — the rewrite's seed component.
 *
 * Exists to prove the whole loop on the clean slate: a component renders
 * in the app, in Storybook, under the unit project, and through the
 * light+dark axe gates. It is NOT an atom of the new design system
 * (GUI-RW 2 owns those); it will be deleted when the first real screen
 * lands.
 */
export function Keel({ title, line }: { title: string; line: string }) {
  return (
    <section aria-label="rebuild status" className="text-center">
      <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
      <p className="mt-2 text-sm text-text-dim">{line}</p>
    </section>
  );
}

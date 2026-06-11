/**
 * Beat — indexing progress in plain words (DESIGN.md §7): a sentence plus
 * an indeterminate sweep. Reduced motion ⇒ static bar; the sentence still
 * updates (values never lie, §2.5).
 */
export function Beat({ sentence }: { sentence: string }) {
  return (
    <div role="status" className="space-y-2">
      <p className="text-sm text-ink">{sentence}</p>
      <div className="h-0.5 overflow-hidden rounded-full bg-sunken">
        <div className="beat-sweep h-full w-2/5 rounded-full bg-brand" />
      </div>
    </div>
  );
}

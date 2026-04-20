import { ReactNode } from 'react';

export function FeatureCard({
  icon,
  title,
  body,
}: {
  icon: ReactNode;
  title: string;
  body: ReactNode;
}) {
  return (
    <div className="group relative rounded-xl border border-fd-border bg-fd-card p-5 transition hover:border-[color-mix(in_srgb,var(--color-iris-400)_40%,transparent)] hover:shadow-md">
      <div className="mb-3 inline-flex size-9 items-center justify-center rounded-lg bg-[color-mix(in_srgb,var(--color-iris-500)_12%,transparent)] text-[var(--color-iris-500)]">
        {icon}
      </div>
      <h3 className="mb-2 text-base font-semibold tracking-tight">{title}</h3>
      <p className="text-sm text-fd-muted-foreground leading-relaxed">{body}</p>
    </div>
  );
}

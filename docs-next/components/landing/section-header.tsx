import { ReactNode } from 'react';

export function SectionHeader({
  eyebrow,
  eyebrowIcon,
  title,
  subtitle,
}: {
  eyebrow: string;
  eyebrowIcon?: ReactNode;
  title: string;
  subtitle?: string;
}) {
  return (
    <div className="mb-10 text-center">
      <span className="inline-flex items-center gap-1.5 rounded-full border border-fd-border bg-fd-card px-3 py-1 text-xs font-mono text-fd-muted-foreground">
        {eyebrowIcon}
        {eyebrow}
      </span>
      <h2 className="mt-4 text-2xl sm:text-3xl font-semibold tracking-tight text-balance">{title}</h2>
      {subtitle && (
        <p className="mx-auto mt-3 max-w-2xl text-sm sm:text-base text-fd-muted-foreground text-balance">
          {subtitle}
        </p>
      )}
    </div>
  );
}

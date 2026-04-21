'use client';

import { useEffect, useRef, useState } from 'react';
import { Check, Copy } from 'lucide-react';

/**
 * CopyButton — tiny clipboard affordance sized to sit next to
 * terminal commands without drawing attention until hovered/focused.
 *
 * Behaviour:
 *   • click → navigator.clipboard.writeText(value)
 *   • on success, swaps the icon to a check and a "copied" aria-live
 *     message for ~1.5s, then reverts
 *   • keyboard accessible (native button, focus-visible ring)
 *   • gracefully no-ops if the Clipboard API is unavailable
 *
 * Styling uses iris tokens so it reads as part of the brand surface
 * in both light and dark themes.
 */
export function CopyButton({
  value,
  className = '',
  label = 'Copy command',
  size = 'md',
}: {
  value: string;
  className?: string;
  label?: string;
  size?: 'sm' | 'md';
}) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    };
  }, []);

  const onCopy = async () => {
    try {
      if (typeof navigator === 'undefined' || !navigator.clipboard) return;
      await navigator.clipboard.writeText(value);
      setCopied(true);
      if (timer.current !== null) window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // Silent: some browsers block clipboard in non-secure contexts.
    }
  };

  const pad = size === 'sm' ? 'p-1' : 'p-1.5';
  const icon = size === 'sm' ? 'size-3' : 'size-3.5';

  return (
    <button
      type="button"
      onClick={onCopy}
      aria-label={copied ? 'Copied to clipboard' : label}
      className={
        'group/copy inline-flex items-center justify-center rounded-md border border-[color-mix(in_oklch,var(--color-iris-400)_20%,transparent)] bg-[color-mix(in_oklch,var(--iris-surface)_50%,transparent)] text-fd-muted-foreground backdrop-blur-sm transition ' +
        'hover:border-[color-mix(in_oklch,var(--color-iris-400)_45%,transparent)] hover:bg-[color-mix(in_oklch,var(--color-iris-500)_14%,transparent)] hover:text-[var(--iris-accent-text)] ' +
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color-mix(in_oklch,var(--color-iris-400)_55%,transparent)] ' +
        pad + ' ' + className
      }
    >
      {copied ? (
        <Check className={icon + ' text-[var(--color-success)]'} aria-hidden />
      ) : (
        <Copy className={icon + ' transition-transform group-hover/copy:scale-110'} aria-hidden />
      )}
      <span className="sr-only" aria-live="polite">
        {copied ? 'Copied to clipboard' : ''}
      </span>
    </button>
  );
}

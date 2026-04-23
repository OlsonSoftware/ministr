'use client';

import 'asciinema-player/dist/bundle/asciinema-player.css';
import { useEffect, useRef } from 'react';

import type { PlayerOptions } from 'asciinema-player';

export interface LaunchPlayerProps {
  /** URL or absolute path to the .cast file. */
  src: string;
  /**
   * Poster frame specifier. Use asciinema's `npt:<mm>:<ss>` form to pin to
   * a specific timestamp, or a data URL for a custom still. Leave
   * undefined for the default (blank terminal at t=0).
   */
  poster?: string;
  /** Additional player options merged over the defaults. */
  options?: Partial<PlayerOptions>;
  /** Extra class on the mount element. */
  className?: string;
  /**
   * Start playing automatically when the player first scrolls into view.
   * Respects `prefers-reduced-motion` — users who opt out of motion
   * see the static poster frame and must click Play themselves.
   */
  autoPlayOnVisible?: boolean;
}

/**
 * LaunchPlayer — mounts an asciinema-player instance over the current
 * `assets/launch.cast` recording.
 *
 * Loads asciinema-player lazily from inside useEffect so the import is
 * deferred past SSG. This component is itself `'use client'` AND is
 * further wrapped in `next/dynamic({ ssr: false })` by its caller
 * (LaunchDemo) — both belt-and-braces are needed because the player's
 * ESM entry touches DOM globals at module-evaluation time.
 */
export default function LaunchPlayer({
  src,
  poster,
  options,
  className,
  autoPlayOnVisible = false,
}: LaunchPlayerProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const instanceRef = useRef<import('asciinema-player').PlayerInstance | null>(null);

  // Serialize the caller's options so we don't re-create the player every
  // render just because the parent passed a fresh object literal.
  const optionsKey = options ? JSON.stringify(options) : '';

  useEffect(() => {
    let cancelled = false;

    (async () => {
      const { create } = await import('asciinema-player');
      if (cancelled || !containerRef.current) return;
      instanceRef.current = create(src, containerRef.current, {
        theme: 'ministr',
        // fit:'both' + an aspect-ratio container keeps the cast at its
        // natural shape. fit:'width' in a narrow column makes the 96
        // cols squeeze to a tiny font and stretches the 50 rows into
        // a ~700px-tall block. Let the container own the shape.
        fit: 'both',
        terminalFontFamily: 'var(--font-mono)',
        terminalFontSize: 'small',
        idleTimeLimit: 2,
        loop: true,
        autoPlay: false,
        controls: 'auto',
        // Preload the cast on mount so the player reports its real
        // dimensions before the user clicks play — no layout jump.
        preload: true,
        ...(poster ? { poster } : {}),
        ...(options ?? {}),
      });
    })().catch((err) => {
      if (!cancelled) console.error('asciinema-player failed to mount', err);
    });

    return () => {
      cancelled = true;
      try {
        instanceRef.current?.dispose();
      } catch {
        /* swallow — unmounting a partially-initialised player is fine */
      }
      instanceRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [src, poster, optionsKey]);

  // Autoplay when the player first enters the viewport — but only for
  // users who haven't opted out of motion. Fires once per mount so a
  // scroll-past-and-back doesn't restart playback mid-session.
  useEffect(() => {
    if (!autoPlayOnVisible) return;
    const el = containerRef.current;
    if (!el) return;

    const reduceMotion =
      typeof window !== 'undefined' &&
      window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    if (reduceMotion) return;

    let triggered = false;
    const obs = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting || triggered) continue;
          triggered = true;
          // Small delay so the player has time to finish mounting
          // + preloading before we call play().
          window.setTimeout(() => {
            try {
              void instanceRef.current?.play();
            } catch {
              /* player not ready yet — user can still click */
            }
          }, 400);
          obs.disconnect();
        }
      },
      { threshold: 0.4 },
    );
    obs.observe(el);
    return () => obs.disconnect();
  }, [autoPlayOnVisible]);

  return (
    <div
      ref={containerRef}
      className={['launch-player', className].filter(Boolean).join(' ')}
      aria-label="ministr + Claude Code terminal recording"
    />
  );
}

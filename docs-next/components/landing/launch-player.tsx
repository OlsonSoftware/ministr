'use client';

import 'asciinema-player/dist/bundle/asciinema-player.css';
import { useEffect, useRef } from 'react';

export interface LaunchPlayerProps {
  /** URL or absolute path to the .cast file. */
  src: string;
  /**
   * Poster frame specifier. Use asciinema's `npt:<mm>:<ss>` form to pin to
   * a specific timestamp, or a data URL for a custom still. Leave
   * undefined for the default (blank terminal at t=0).
   */
  poster?: string;
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
export default function LaunchPlayer({ src, poster }: LaunchPlayerProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    let instance: import('asciinema-player').PlayerInstance | null = null;

    (async () => {
      const { create } = await import('asciinema-player');
      if (cancelled || !containerRef.current) return;
      instance = create(src, containerRef.current, {
        theme: 'dracula',
        fit: 'width',
        terminalFontFamily: 'var(--font-mono)',
        terminalFontSize: 'medium',
        idleTimeLimit: 2,
        loop: true,
        autoPlay: false,
        controls: 'auto',
        ...(poster ? { poster } : {}),
      });
    })().catch((err) => {
      if (!cancelled) console.error('asciinema-player failed to mount', err);
    });

    return () => {
      cancelled = true;
      try {
        instance?.dispose();
      } catch {
        /* swallow — unmounting a partially-initialised player is fine */
      }
    };
  }, [src, poster]);

  return (
    <div
      ref={containerRef}
      className="launch-player"
      aria-label="ministr + Claude Code terminal recording"
    />
  );
}

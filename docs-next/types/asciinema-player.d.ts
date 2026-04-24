// Minimal ambient typings for asciinema-player — the npm package ships
// no declaration file. Covers the slice of the API we actually call in
// `components/landing/launch-player.tsx`.
//
// See https://docs.asciinema.org/manual/player/options/ for the full
// option reference.

declare module 'asciinema-player' {
  export interface PlayerOptions {
    theme?: string;
    fit?: 'width' | 'height' | 'both' | false;
    terminalFontFamily?: string;
    terminalFontSize?: 'small' | 'medium' | 'big' | string;
    terminalLineHeight?: number;
    cols?: number;
    rows?: number;
    idleTimeLimit?: number;
    loop?: boolean | number;
    autoPlay?: boolean;
    controls?: boolean | 'auto';
    startAt?: number | string;
    speed?: number;
    poster?: string;
    preload?: boolean;
    pauseOnMarkers?: boolean;
    markers?: Array<
      number | [number, string] | { time: number; label?: string }
    >;
  }

  export interface PlayerInstance {
    play: () => Promise<void>;
    pause: () => void;
    seek: (pos: number | string) => Promise<void>;
    getCurrentTime: () => number;
    getDuration: () => number | null;
    dispose: () => void;
    addEventListener: (event: string, handler: (...args: unknown[]) => void) => void;
    removeEventListener: (event: string, handler: (...args: unknown[]) => void) => void;
  }

  export function create(
    src: string | { url: string; data?: unknown },
    element: HTMLElement,
    opts?: PlayerOptions,
  ): PlayerInstance;
}

declare module 'asciinema-player/dist/bundle/asciinema-player.css';

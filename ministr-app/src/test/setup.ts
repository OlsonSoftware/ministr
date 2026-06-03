/**
 * Vitest setup — jest-dom matchers + the browser-API polyfills happy-dom
 * doesn't ship, plus a default no-op Tauri IPC mock so any stray `invoke`
 * during a render resolves to `null` instead of throwing. Individual tests
 * install richer fixtures with `mockIPC` (see @tauri-apps/api/mocks).
 */
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach } from "vitest";
import { cleanup } from "@testing-library/react";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";

// ResizeObserver — used by AdaptiveSurface + a few atoms; happy-dom omits it.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver ??=
  ResizeObserverStub as unknown as typeof ResizeObserver;

// matchMedia — read by theme/density hooks.
globalThis.matchMedia ??= ((query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: () => {},
  removeListener: () => {},
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: () => false,
})) as unknown as typeof globalThis.matchMedia;

beforeEach(() => {
  // Default: every Tauri command resolves to null unless a test overrides it.
  mockIPC(() => null);
});

afterEach(() => {
  cleanup();
  clearMocks();
  localStorage.clear();
});

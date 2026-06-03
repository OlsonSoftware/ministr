import type { Decorator } from "@storybook/react-vite";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";

/**
 * Storybook Tauri-mock harness — lets surfaces that call `invoke(...)` render
 * in Storybook (which has no Tauri runtime) by intercepting the IPC with
 * `@tauri-apps/api/mocks`. Fixtures are keyed by command name; a value can be
 * the response directly or a `(args) => response` function. Unmocked commands
 * fall back to `null` (which most read paths treat as "empty").
 *
 * Usage:
 *   const meta = { decorators: [withTauriMock(DEMO_FIXTURES)] }
 */
export type TauriFixtures = Record<
  string,
  unknown | ((args: Record<string, unknown>) => unknown)
>;

export function withTauriMock(fixtures: TauriFixtures = {}): Decorator {
  const decorator: Decorator = (Story) => {
    // Must be installed synchronously during render, before the story's
    // children run their mount-time `invoke` effects.
    mockIPC((cmd, args) => {
      const f = fixtures[cmd];
      if (typeof f === "function") {
        return (f as (a: Record<string, unknown>) => unknown)(
          (args ?? {}) as Record<string, unknown>,
        );
      }
      return f ?? null;
    });
    return <Story />;
  };
  return decorator;
}

/** Tear down the IPC mock — attach as a play/cleanup if a story needs it. */
export function clearTauriMock(): void {
  clearMocks();
}

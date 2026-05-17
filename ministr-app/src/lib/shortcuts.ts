/**
 * Single source of truth for keyboard shortcuts.
 *
 * Three consumers read from this map:
 *   - `App.tsx` — global keydown handler (matches via `matchShortcut`).
 *   - `CommandPalette.tsx` — joins `keys` into the per-row kbd hint.
 *   - `ShortcutSheet.tsx` — renders the cheat-sheet, grouped by `group`.
 *
 * Adding or renaming a binding requires editing exactly one place.
 *
 * The `action` tag is a discriminator that the global handler maps to a
 * concrete callback. Adding a new action means: declare it here + handle
 * it in App.tsx's switch statement. CommandPalette consumes the same tag
 * so palette rows that mirror a global shortcut can show the kbd hint
 * without duplicating the keys array.
 */

export type ShortcutAction =
  // navigation — three surfaces only after the M1 IA collapse.
  | "nav:ask"
  | "nav:projects"
  | "nav:sessions"
  | "nav:settings"
  // chrome toggles
  | "toggle:palette"
  | "toggle:shortcuts";

export type ShortcutGroup = "Navigation" | "Interface";

export interface ShortcutDef {
  id: ShortcutAction;
  /** Display form. First element of a 2-tuple is the prefix (e.g. "g"). */
  keys: string[];
  /** Sentence-case description shown in the cheat sheet + palette hint. */
  label: string;
  group: ShortcutGroup;
  /** True when this binding is the `g {x}` two-keystroke pattern. */
  prefixed?: boolean;
  /** True when this binding should still fire when the user is typing
   *  inside an `<input>` / `<textarea>` / contenteditable. Reserved for
   *  the truly-global escape hatches (the command palette ⌘K / Ctrl+K),
   *  not for letter-based bindings that would conflict with normal text
   *  entry. App.tsx checks this flag before its typing-bail. */
  firesWhileTyping?: boolean;
}

export const SHORTCUTS: readonly ShortcutDef[] = [
  // ── Chrome toggles ──────────────────────────────────────────────────
  {
    id: "toggle:palette",
    keys: ["⌘", "K"],
    label: "Open command palette",
    group: "Navigation",
    firesWhileTyping: true,
  },
  {
    id: "toggle:shortcuts",
    keys: ["?"],
    label: "Show this sheet",
    group: "Interface",
  },
  // ── g-prefixed navigation ───────────────────────────────────────────
  {
    id: "nav:ask",
    keys: ["g", "a"],
    label: "Ask",
    group: "Navigation",
    prefixed: true,
  },
  {
    id: "nav:projects",
    keys: ["g", "p"],
    label: "Projects",
    group: "Navigation",
    prefixed: true,
  },
  {
    id: "nav:sessions",
    keys: ["g", "s"],
    label: "Sessions",
    group: "Navigation",
    prefixed: true,
  },
  {
    id: "nav:settings",
    keys: ["g", ","],
    label: "Settings",
    group: "Navigation",
    prefixed: true,
  },
];

/** Lookup index by action id — built once, used by every consumer. */
const BY_ID = new Map<ShortcutAction, ShortcutDef>(
  SHORTCUTS.map((s) => [s.id, s]),
);

export function getShortcut(id: ShortcutAction): ShortcutDef | undefined {
  return BY_ID.get(id);
}

/** Get the display-form keys for an action, or [] if unbound. */
export function shortcutKeys(id: ShortcutAction): string[] {
  return BY_ID.get(id)?.keys ?? [];
}

/** True when an action should fire even while focus is in an input. */
export function firesWhileTyping(id: ShortcutAction): boolean {
  return BY_ID.get(id)?.firesWhileTyping ?? false;
}

/** Group definitions for the cheat sheet. */
export function shortcutGroups(): { title: ShortcutGroup; items: ShortcutDef[] }[] {
  const groups = new Map<ShortcutGroup, ShortcutDef[]>();
  for (const s of SHORTCUTS) {
    const list = groups.get(s.group) ?? [];
    list.push(s);
    groups.set(s.group, list);
  }
  // Preserve insertion order: Navigation first, Interface second.
  const order: ShortcutGroup[] = ["Navigation", "Interface"];
  return order
    .filter((g) => groups.has(g))
    .map((title) => ({ title, items: groups.get(title)! }));
}

/**
 * Match an actual KeyboardEvent against the shortcut map.
 *
 * Two-state design:
 *   - For `prefixed` shortcuts the caller passes `gPending=true` after the
 *     user just hit `g`; this function returns the matched action and the
 *     caller resets pending. For first-keystroke `g`, returns the special
 *     `_pending:g` sentinel so the caller knows to start the timer.
 *   - For chrome shortcuts (⌘K, ?) it matches directly.
 *
 * Returns the matched action id or null.
 */
export type MatchResult = ShortcutAction | "_pending:g" | null;

export function matchShortcut(
  e: KeyboardEvent,
  gPending: boolean,
): MatchResult {
  // ⌘K / Ctrl+K — works while typing.
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
    return "toggle:palette";
  }

  // The remaining shortcuts only fire when not typing — caller checks.

  // ? (or shift+/) → cheat sheet.
  if (e.key === "?" || (e.key === "/" && e.shiftKey)) {
    return "toggle:shortcuts";
  }

  // Two-stroke `g {x}` navigation.
  if (gPending) {
    const second = e.key.toLowerCase();
    for (const s of SHORTCUTS) {
      if (!s.prefixed) continue;
      if (s.keys[1].toLowerCase() === second) return s.id;
    }
    return null; // pending but no match — caller resets.
  }
  if (e.key.toLowerCase() === "g") {
    return "_pending:g";
  }

  return null;
}

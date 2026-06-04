/**
 * Resolve the concrete Shiki colour scheme from the *rendered* surface.
 *
 * The single source of truth is the `.dark` class on <html> (set by useTheme
 * from the `system | dark | light` preference). Reading the class — rather than
 * the preference directly — guarantees syntax highlighting always matches the
 * surface it sits on, including in Storybook where the theme decorator toggles
 * the class independently of the app's theme state. A MutationObserver keeps it
 * live when the theme flips.
 */
import { useEffect, useState } from "react";

export type ColorScheme = "dark" | "light";

function readScheme(): ColorScheme {
  if (typeof document === "undefined") return "dark";
  return document.documentElement.classList.contains("dark") ? "dark" : "light";
}

export function useColorScheme(): ColorScheme {
  const [scheme, setScheme] = useState<ColorScheme>(readScheme);

  useEffect(() => {
    const root = document.documentElement;
    const obs = new MutationObserver(() => setScheme(readScheme()));
    obs.observe(root, { attributes: true, attributeFilter: ["class"] });
    // Sync once on mount in case the class changed before the observer attached.
    setScheme(readScheme());
    return () => obs.disconnect();
  }, []);

  return scheme;
}

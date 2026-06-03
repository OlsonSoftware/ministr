/**
 * Resolve the live colour scheme straight from the `.dark` class on
 * `<html>` — the single source of truth the app (and Storybook's theme
 * toggle) both flip. Unlike `useColorScheme`, this needs no ThemeProvider,
 * so it works in any rendering context (stories, isolated previews).
 */
import { useEffect, useState } from "react";
import type { ColorScheme } from "../components/code/useColorScheme";

function read(): ColorScheme {
  if (typeof document === "undefined") return "light";
  return document.documentElement.classList.contains("dark") ? "dark" : "light";
}

export function useDocumentScheme(): ColorScheme {
  const [scheme, setScheme] = useState<ColorScheme>(read);

  useEffect(() => {
    const el = document.documentElement;
    const obs = new MutationObserver(() => setScheme(read()));
    obs.observe(el, { attributes: true, attributeFilter: ["class"] });
    return () => obs.disconnect();
  }, []);

  return scheme;
}

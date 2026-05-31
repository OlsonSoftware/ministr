/**
 * Resolve the app's theme preference to a concrete Shiki colour scheme.
 *
 * The preference is `system | dark | light`; Shiki needs a concrete
 * `dark | light`. For `system` we follow `prefers-color-scheme` and react to
 * OS-level changes live.
 */
import { useEffect, useState } from "react";
import { useTheme } from "../../hooks/useTheme";

export type ColorScheme = "dark" | "light";

function systemPrefersDark(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches
  );
}

export function useColorScheme(): ColorScheme {
  const { theme } = useTheme();
  const [systemDark, setSystemDark] = useState(systemPrefersDark);

  useEffect(() => {
    if (theme !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [theme]);

  if (theme === "dark") return "dark";
  if (theme === "light") return "light";
  return systemDark ? "dark" : "light";
}

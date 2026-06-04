/**
 * Shiki highlighting, as a hook.
 *
 * One process-wide highlighter is created lazily and shared (creating one per
 * file would re-load grammars + themes every navigation). Languages load on
 * demand; an unknown/unsupported language falls back to plain text rather than
 * throwing. The same Shiki engine VS Code uses gives us TextMate-accurate
 * highlighting for 100+ languages.
 */
import { useEffect, useState } from "react";
import {
  createHighlighter,
  type DecorationItem,
  type Highlighter,
} from "shiki";
import type { ColorScheme } from "./useColorScheme";

const THEME_DARK = "github-dark-default";
// High-contrast light theme: github-light-default's comment/keyword tokens only
// reach ~4.1:1 on our tinted code surfaces (they're tuned for pure white), which
// trips the a11y gate. The high-contrast variant clears 4.5:1 on the inset
// surface while keeping the familiar GitHub palette.
const THEME_LIGHT = "github-light-high-contrast";

let highlighterPromise: Promise<Highlighter> | null = null;

function getHighlighter(): Promise<Highlighter> {
  if (!highlighterPromise) {
    highlighterPromise = createHighlighter({
      themes: [THEME_DARK, THEME_LIGHT],
      langs: [],
    });
  }
  return highlighterPromise;
}

/** Ensure `lang` is loaded; return the lang actually usable (`"text"` fallback). */
async function ensureLang(h: Highlighter, lang: string): Promise<string> {
  if (lang === "text" || lang === "plaintext" || lang === "") return "text";
  if (h.getLoadedLanguages().includes(lang)) return lang;
  try {
    await h.loadLanguage(lang as Parameters<Highlighter["loadLanguage"]>[0]);
    return lang;
  } catch {
    return "text";
  }
}

export interface HighlightRequest {
  code: string;
  lang: string;
  scheme: ColorScheme;
  decorations: DecorationItem[];
}

export interface HighlightState {
  html: string | null;
  loading: boolean;
  error: string | null;
}

export function useHighlightedHtml(req: HighlightRequest | null): HighlightState {
  const [state, setState] = useState<HighlightState>({
    html: null,
    loading: req !== null,
    error: null,
  });

  // Decorations are derived data; a stable key avoids re-highlighting on
  // referentially-new-but-equal arrays.
  const decorationsKey = req
    ? req.decorations
        .map((d) => {
          const s = d.start as { line: number; character: number };
          const id = (d.properties?.["data-symbol-id"] as string) ?? "";
          return `${s.line}:${s.character}:${id}`;
        })
        .join("|")
    : "";

  useEffect(() => {
    if (!req) {
      setState({ html: null, loading: false, error: null });
      return;
    }
    let cancelled = false;
    setState((prev) => ({ ...prev, loading: true, error: null }));
    (async () => {
      try {
        const h = await getHighlighter();
        const lang = await ensureLang(h, req.lang);
        const html = h.codeToHtml(req.code, {
          lang,
          theme: req.scheme === "dark" ? THEME_DARK : THEME_LIGHT,
          decorations: req.decorations,
          // github-light-high-contrast's comment token (#66707b) only reaches
          // 4.0:1 on the inset code surface; darken it to clear the 4.5 AA floor.
          colorReplacements:
            req.scheme === "dark" ? undefined : { "#66707b": "#586069" },
        });
        if (!cancelled) setState({ html, loading: false, error: null });
      } catch (e) {
        if (!cancelled)
          setState({ html: null, loading: false, error: String(e) });
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [req?.code, req?.lang, req?.scheme, decorationsKey]);

  return state;
}

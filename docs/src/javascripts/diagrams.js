// Interactive D2 diagrams. The mkdocs-d2-plugin renders each diagram as
// an inline <svg> wrapped in <div class="iris-diagram">. This module
// walks the rendered nodes, adds hover highlight styling, and — when a
// node's title matches a known iris_* tool — wires a click handler that
// navigates to the tool's reference page.

(function () {
  "use strict";

  const TOOL_NAMES = new Set([
    "iris_survey",
    "iris_read",
    "iris_extract",
    "iris_related",
    "iris_toc",
    "iris_symbols",
    "iris_definition",
    "iris_references",
    "iris_bridge",
    "iris_budget",
    "iris_compress",
    "iris_evicted",
    "iris_fetch",
    "iris_clone",
    "iris_refresh",
  ]);

  const SITE_BASE =
    document.querySelector("meta[name='iris:site-base']")?.content || "./";

  function toolUrl(name) {
    const short = name.replace(/^iris_/, "");
    try {
      return new URL(
        `tools/${short}/`,
        new URL(SITE_BASE, window.location.href),
      ).href;
    } catch (_) {
      return `tools/${short}/`;
    }
  }

  function extractTitleText(node) {
    // D2 emits node labels as a nested <text> element rather than <title>.
    // We look for the first text child (direct or grandchild) that isn't
    // empty. Pure layout groups (class="shape") contain no text.
    const titleEl = node.querySelector(":scope > title");
    if (titleEl && titleEl.textContent) return titleEl.textContent.trim();
    const textEl = node.querySelector(":scope > text, :scope > g > text");
    if (textEl && textEl.textContent) return textEl.textContent.trim();
    return null;
  }

  function findToolName(text) {
    if (!text) return null;
    const match = text.match(/iris_[a-z]+/);
    return match && TOOL_NAMES.has(match[0]) ? match[0] : null;
  }

  function enhanceDiagram(diagram) {
    if (diagram.dataset.irisEnhanced === "1") return;
    diagram.dataset.irisEnhanced = "1";
    diagram.classList.add("iris-diagram--interactive");

    const svgs = diagram.querySelectorAll("svg");
    svgs.forEach((svg) => {
      // d2 emits each node as a <g class="…shape…">, often with a nested
      // <title>. We key off presence of <title> to avoid labelling random
      // layout groups.
      const nodes = svg.querySelectorAll("g");
      nodes.forEach((g) => {
        const text = extractTitleText(g);
        if (!text) return;
        g.classList.add("iris-diagram__node");
        const tool = findToolName(text);
        if (tool) {
          g.classList.add("iris-diagram__node--tool");
          g.setAttribute("role", "link");
          g.setAttribute("tabindex", "0");
          g.setAttribute("aria-label", `Open ${tool} reference`);
          const navigate = () => {
            window.location.assign(toolUrl(tool));
          };
          g.addEventListener("click", (e) => {
            e.preventDefault();
            navigate();
          });
          g.addEventListener("keydown", (e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              navigate();
            }
          });
        }
      });
    });
  }

  function hydrate() {
    document
      .querySelectorAll(".iris-diagram, .d2, div.d2")
      .forEach((el) => enhanceDiagram(el));
    // Also match any diagram that simply contains an inline svg but wasn't
    // wrapped (mkdocs-d2-plugin defaults).
    document.querySelectorAll("p > svg, figure > svg").forEach((svg) => {
      enhanceDiagram(svg.parentElement);
    });
  }

  if (typeof window !== "undefined") {
    if (window.document$ && typeof window.document$.subscribe === "function") {
      window.document$.subscribe(hydrate);
    } else if (document.readyState !== "loading") {
      hydrate();
    } else {
      document.addEventListener("DOMContentLoaded", hydrate);
    }
  }
})();

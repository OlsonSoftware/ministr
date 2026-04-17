// Keyboard shortcuts + '?' cheatsheet for the iris docs.
//
// Vim-style `g X` prefix jumps (g h home, g t tools, g a architecture,
// g b benchmarks), `[` / `]` step through Material's prev/next pagination
// links, and `?` toggles a cheatsheet overlay. All suppressed when focus
// is inside an input / textarea / contenteditable or while the command
// palette is open.

(function () {
  "use strict";

  const SITE_BASE = document.querySelector("meta[name='iris:site-base']")?.content || "./";

  function url(rel) {
    try {
      return new URL(rel, new URL(SITE_BASE, window.location.href)).href;
    } catch (_) {
      return rel;
    }
  }

  const JUMPS = {
    h: { label: "Home", target: () => url("./") },
    t: { label: "Tool reference", target: () => url("tools/") },
    a: { label: "Architecture", target: () => url("architecture/") },
    b: { label: "Benchmarks", target: () => url("benchmarks/") },
    c: { label: "Configuration", target: () => url("configuration/") },
    s: { label: "Getting started", target: () => url("getting-started/") },
  };

  const SHORTCUTS = [
    { keys: ["⌘", "K"], label: "Open command palette" },
    { keys: ["g", "h"], label: "Go to Home" },
    { keys: ["g", "t"], label: "Go to Tool reference" },
    { keys: ["g", "a"], label: "Go to Architecture" },
    { keys: ["g", "b"], label: "Go to Benchmarks" },
    { keys: ["g", "c"], label: "Go to Configuration" },
    { keys: ["g", "s"], label: "Go to Getting started" },
    { keys: ["["], label: "Previous page" },
    { keys: ["]"], label: "Next page" },
    { keys: ["?"], label: "Show this cheatsheet" },
    { keys: ["Esc"], label: "Close dialogs" },
  ];

  function isTypingTarget(target) {
    if (!target) return false;
    const tag = target.tagName;
    return (
      tag === "INPUT" ||
      tag === "TEXTAREA" ||
      target.isContentEditable
    );
  }

  function paletteOpen() {
    return !!document.querySelector(".iris-palette.is-open");
  }

  function clickMaterialNav(selector) {
    const link = document.querySelector(selector);
    if (link && link.href) window.location.assign(link.href);
  }

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }

  let cheatsheetEl = null;
  function buildCheatsheet() {
    const backdrop = el("div", "iris-cheatsheet");
    backdrop.setAttribute("role", "dialog");
    backdrop.setAttribute("aria-modal", "true");
    backdrop.setAttribute("aria-label", "Keyboard shortcuts");

    const panel = el("div", "iris-cheatsheet__panel");
    const header = el("div", "iris-cheatsheet__header");
    const title = el("h2", "iris-cheatsheet__title", "Keyboard shortcuts");
    const dismiss = el("button", "iris-cheatsheet__dismiss");
    dismiss.type = "button";
    dismiss.setAttribute("aria-label", "Close");
    dismiss.innerHTML =
      '<svg viewBox="0 0 256 256" width="16" height="16" aria-hidden="true"><path fill="currentColor" d="m205.66 194.34l-11.32 11.32L128 139.31l-66.34 66.35l-11.32-11.32L116.69 128L50.34 61.66l11.32-11.32L128 116.69l66.34-66.35l11.32 11.32L139.31 128Z"/></svg>';
    header.appendChild(title);
    header.appendChild(dismiss);

    const grid = el("div", "iris-cheatsheet__grid");
    for (const sc of SHORTCUTS) {
      const row = el("div", "iris-cheatsheet__row");
      const keys = el("div", "iris-cheatsheet__keys");
      const groupHtml = (list) =>
        `<span class='iris-cheatsheet__group-keys'>${list
          .map((k) => `<kbd>${k}</kbd>`)
          .join("<span class='iris-cheatsheet__plus'>+</span>")}</span>`;
      let html = groupHtml(sc.keys);
      if (sc.altKeys) {
        html += `<span class='iris-cheatsheet__or'>or</span>${groupHtml(sc.altKeys)}`;
      }
      keys.innerHTML = html;
      const label = el("div", "iris-cheatsheet__label", sc.label);
      row.appendChild(keys);
      row.appendChild(label);
      grid.appendChild(row);
    }

    const footer = el(
      "div",
      "iris-cheatsheet__footer",
      "On Windows / Linux, use Ctrl where ⌘ is shown. Type letter pairs within 1 s. Press ? again or Esc to close.",
    );

    panel.appendChild(header);
    panel.appendChild(grid);
    panel.appendChild(footer);
    backdrop.appendChild(panel);

    backdrop.addEventListener("click", (e) => {
      if (e.target === backdrop) hideCheatsheet();
    });
    dismiss.addEventListener("click", hideCheatsheet);

    return backdrop;
  }

  function showCheatsheet() {
    if (cheatsheetEl) return;
    cheatsheetEl = buildCheatsheet();
    document.body.appendChild(cheatsheetEl);
    requestAnimationFrame(() => cheatsheetEl.classList.add("is-open"));
    document.documentElement.style.overflow = "hidden";
  }

  function hideCheatsheet() {
    if (!cheatsheetEl) return;
    const node = cheatsheetEl;
    cheatsheetEl = null;
    node.classList.remove("is-open");
    document.documentElement.style.overflow = "";
    setTimeout(() => {
      if (node.parentNode) node.parentNode.removeChild(node);
    }, 150);
  }

  function toggleCheatsheet() {
    cheatsheetEl ? hideCheatsheet() : showCheatsheet();
  }

  let gPending = false;
  let gTimer = null;

  if (typeof window !== "undefined") {
    // Capture phase so our `?` / `g _` / `[` / `]` shortcuts run before
    // Material's built-in bindings (which also grab `?` for the search
    // input). We only preventDefault + stopPropagation for keys we
    // actually handle, so normal typing is untouched.
    document.addEventListener(
      "keydown",
      (e) => {
        const handled = maybeHandle(e);
        if (handled) {
          e.preventDefault();
          e.stopPropagation();
        }
      },
      true,
    );
  }

  function maybeHandle(e) {
    if (paletteOpen()) return false;
    if (e.metaKey || e.ctrlKey || e.altKey) return false;

    if (e.key === "Escape") {
      if (cheatsheetEl) {
        hideCheatsheet();
        return true;
      }
      return false;
    }

    // `?` is intercepted before the typing-target check because Material's
    // search input auto-focuses on `?` — by the time our handler runs,
    // e.target is already the search input, which would otherwise suppress
    // us. Also accept Shift+/ since some browsers report e.key as "/" with
    // shiftKey=true rather than the composed "?".
    if (e.key === "?" || (e.key === "/" && e.shiftKey)) {
      toggleCheatsheet();
      return true;
    }

    if (isTypingTarget(e.target)) return false;

    if (cheatsheetEl) return false;

    if (e.key === "[") {
      clickMaterialNav(".md-footer__link--prev");
      return true;
    }
    if (e.key === "]") {
      clickMaterialNav(".md-footer__link--next");
      return true;
    }

    if (gPending) {
      const jump = JUMPS[e.key.toLowerCase()];
      clearTimeout(gTimer);
      gPending = false;
      if (jump) {
        window.location.assign(jump.target());
        return true;
      }
      return false;
    }

    if (e.key === "g") {
      gPending = true;
      gTimer = setTimeout(() => {
        gPending = false;
      }, 1000);
      return true;
    }

    return false;
  }
})();

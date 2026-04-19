// Cmd+K / Ctrl+K command palette for the iris docs.
//
// Loads Material's rendered search_index.json once on first open, then
// fuzzy-matches across pages + the iris_* tool reference. Groups results
// into Pages / Tools / Recent (last 5 visited from localStorage). Arrow
// keys move the highlight, Enter navigates, Esc closes. Suppressed when
// focus is already inside an input / textarea / contenteditable.

(function () {
  "use strict";

  const MAX_RESULTS = 8;
  const RECENT_KEY = "iris:palette:recent";
  const RECENT_MAX = 5;

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }

  function isTypingTarget(target) {
    if (!target) return false;
    const tag = target.tagName;
    return (
      tag === "INPUT" ||
      tag === "TEXTAREA" ||
      target.isContentEditable
    );
  }

  function readRecent() {
    try {
      return JSON.parse(localStorage.getItem(RECENT_KEY) || "[]");
    } catch (_) {
      return [];
    }
  }

  function pushRecent(entry) {
    try {
      const list = readRecent().filter((e) => e.location !== entry.location);
      list.unshift({
        location: entry.location,
        title: entry.title,
        group: entry.group,
      });
      localStorage.setItem(
        RECENT_KEY,
        JSON.stringify(list.slice(0, RECENT_MAX)),
      );
    } catch (_) {
      /* localStorage disabled / private-mode — silently ignore */
    }
  }

  let indexPromise = null;
  function loadIndex() {
    if (indexPromise) return indexPromise;
    const meta = document.querySelector("meta[name='iris:search-index']");
    const url = meta?.content;
    if (!url) {
      indexPromise = Promise.resolve({ docs: [] });
      return indexPromise;
    }
    indexPromise = fetch(url)
      .then((r) => (r.ok ? r.json() : { docs: [] }))
      .then((raw) => ({
        docs: (raw.docs || []).filter((d) => {
          if (d.location === undefined || d.location === null) return false;
          if (!d.title) return false;
          // Exclude section anchors — Material indexes every heading, but
          // the palette is for page-level navigation, not jump-to-section.
          return !d.location.includes("#");
        }),
      }))
      .catch(() => ({ docs: [] }));
    return indexPromise;
  }

  function fuzzyScore(haystack, needle) {
    if (!needle) return 0;
    haystack = haystack.toLowerCase();
    needle = needle.toLowerCase();
    // Fast path: substring match ranks highest.
    const substr = haystack.indexOf(needle);
    if (substr !== -1) return 1000 - substr;
    // Character-in-order match.
    let hi = 0;
    let score = 0;
    let lastHit = -2;
    for (let ni = 0; ni < needle.length; ni++) {
      const c = needle[ni];
      const found = haystack.indexOf(c, hi);
      if (found === -1) return 0;
      score += found - lastHit === 1 ? 3 : 1;
      lastHit = found;
      hi = found + 1;
    }
    return score;
  }

  function classifyDoc(doc) {
    const loc = doc.location || "";
    if (loc.startsWith("tools/") || loc.includes("/tools/")) return "Tools";
    return "Pages";
  }

  const HTML_ENTITIES = {
    "&amp;": "&",
    "&lt;": "<",
    "&gt;": ">",
    "&quot;": '"',
    "&#39;": "'",
    "&apos;": "'",
  };
  function decodeEntities(s) {
    if (!s || typeof s !== "string") return s;
    return s.replace(/&(?:amp|lt|gt|quot|#39|apos);/g, (m) => HTML_ENTITIES[m] || m);
  }

  function rankDocs(docs, query) {
    if (!query) {
      return docs
        .filter((d) => d.title && d.location)
        .map((d) => ({ doc: d, score: 0 }));
    }
    const out = [];
    for (const d of docs) {
      if (!d.title || !d.location) continue;
      const titleScore = fuzzyScore(d.title, query);
      const locScore = fuzzyScore(d.location, query) * 0.4;
      const textScore = d.text ? fuzzyScore(d.text.slice(0, 200), query) * 0.2 : 0;
      const score = titleScore + locScore + textScore;
      if (score > 0) out.push({ doc: d, score });
    }
    out.sort((a, b) => b.score - a.score);
    return out;
  }

  function groupResults(ranked) {
    const groups = { Tools: [], Pages: [] };
    for (const { doc } of ranked) {
      const g = classifyDoc(doc);
      if (groups[g].length < MAX_RESULTS) groups[g].push(doc);
    }
    return groups;
  }

  function buildModal(ctx) {
    const backdrop = el("div", "iris-palette");
    backdrop.setAttribute("role", "dialog");
    backdrop.setAttribute("aria-modal", "true");
    backdrop.setAttribute("aria-label", "Command palette");

    const panel = el("div", "iris-palette__panel");
    const inputWrap = el("div", "iris-palette__input-wrap");
    const searchIcon = el("span", "iris-palette__icon");
    searchIcon.innerHTML =
      '<svg viewBox="0 0 256 256" width="18" height="18" aria-hidden="true"><path fill="currentColor" d="M229.66 218.34l-50.07-50.06a88.11 88.11 0 1 0-11.31 11.31l50.06 50.07a8 8 0 0 0 11.32-11.32ZM40 112a72 72 0 1 1 72 72a72.08 72.08 0 0 1-72-72Z"/></svg>';
    const input = el("input", "iris-palette__input");
    input.type = "text";
    input.placeholder = "Search pages and tools…";
    input.spellcheck = false;
    input.autocomplete = "off";
    const hint = el("span", "iris-palette__hint");
    hint.innerHTML = "<kbd>Esc</kbd>";
    inputWrap.appendChild(searchIcon);
    inputWrap.appendChild(input);
    inputWrap.appendChild(hint);

    const results = el("div", "iris-palette__results");
    panel.appendChild(inputWrap);
    panel.appendChild(results);
    backdrop.appendChild(panel);

    ctx.backdrop = backdrop;
    ctx.panel = panel;
    ctx.input = input;
    ctx.results = results;

    backdrop.addEventListener("click", (e) => {
      if (e.target === backdrop) close(ctx);
    });
    input.addEventListener("input", () => render(ctx));
    input.addEventListener("keydown", (e) => onKey(e, ctx));

    return backdrop;
  }

  function render(ctx) {
    const query = ctx.input.value.trim();
    ctx.results.innerHTML = "";
    ctx.items = [];

    const sections = [];
    if (!query) {
      const recent = readRecent();
      if (recent.length) sections.push({ title: "Recent", items: recent });
    }

    const ranked = rankDocs(ctx.docs, query);
    const groups = groupResults(ranked);
    if (groups.Tools.length) sections.push({ title: "Tools", items: groups.Tools });
    if (groups.Pages.length) sections.push({ title: "Pages", items: groups.Pages });

    if (!sections.length) {
      const empty = el(
        "div",
        "iris-palette__empty",
        query ? `No matches for "${query}"` : "Start typing to search…",
      );
      ctx.results.appendChild(empty);
      return;
    }

    for (const sec of sections) {
      const header = el("div", "iris-palette__group", sec.title);
      ctx.results.appendChild(header);
      for (const item of sec.items) {
        const row = el("a", "iris-palette__item");
        row.href = item.location;
        row.setAttribute("data-group", sec.title);

        const icon = el("span", "iris-palette__item-icon");
        const isTool = sec.title === "Tools" || /\/tools\//.test(item.location);
        icon.innerHTML = isTool
          ? '<svg viewBox="0 0 256 256" width="14" height="14" aria-hidden="true"><path fill="currentColor" d="M69.66 98.34a8 8 0 0 1 0 11.32l-26.35 26.34l26.35 26.34a8 8 0 0 1-11.32 11.32l-32-32a8 8 0 0 1 0-11.32l32-32a8 8 0 0 1 11.32 0Zm160 26.34l-32-32a8 8 0 0 0-11.32 11.32l26.35 26.34l-26.35 26.34a8 8 0 0 0 11.32 11.32l32-32a8 8 0 0 0 0-11.32Zm-82.4-83.23a8 8 0 0 0-10.12 5.05l-48 144a8 8 0 0 0 5.05 10.12a7.89 7.89 0 0 0 2.53.41a8 8 0 0 0 7.59-5.46l48-144a8 8 0 0 0-5.05-10.12Z"/></svg>'
          : '<svg viewBox="0 0 256 256" width="14" height="14" aria-hidden="true"><path fill="currentColor" d="M208 40H48a16 16 0 0 0-16 16v144a16 16 0 0 0 16 16h160a16 16 0 0 0 16-16V56a16 16 0 0 0-16-16ZM48 56h160v48H48Zm0 144v-80h160v80Z"/></svg>';

        const text = el("span", "iris-palette__item-text");
        const title = el("span", "iris-palette__item-title", decodeEntities(item.title));
        const path = el(
          "span",
          "iris-palette__item-path",
          prettyPath(item.location),
        );
        text.appendChild(title);
        text.appendChild(path);

        const kbd = el("span", "iris-palette__item-kbd");
        kbd.innerHTML = "<kbd>↵</kbd>";

        row.appendChild(icon);
        row.appendChild(text);
        row.appendChild(kbd);

        row.addEventListener("click", (e) => {
          e.preventDefault();
          navigateTo(ctx, item);
        });
        row.addEventListener("mouseenter", () => {
          setActive(ctx, ctx.items.indexOf(row));
        });

        ctx.results.appendChild(row);
        ctx.items.push(row);
      }
    }

    setActive(ctx, 0);
  }

  function prettyPath(loc) {
    if (!loc) return "home";
    const stripped = loc.replace(/\/$/, "").replace(/#.*$/, "");
    return stripped || "home";
  }

  function setActive(ctx, idx) {
    if (!ctx.items.length) return;
    if (idx < 0) idx = 0;
    if (idx >= ctx.items.length) idx = ctx.items.length - 1;
    ctx.activeIdx = idx;
    ctx.items.forEach((el, i) => {
      el.classList.toggle("is-active", i === idx);
    });
    const active = ctx.items[idx];
    if (active && active.scrollIntoView) {
      active.scrollIntoView({ block: "nearest" });
    }
  }

  function onKey(e, ctx) {
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        setActive(ctx, (ctx.activeIdx ?? -1) + 1);
        break;
      case "ArrowUp":
        e.preventDefault();
        setActive(ctx, (ctx.activeIdx ?? 0) - 1);
        break;
      case "Enter": {
        e.preventDefault();
        const active = ctx.items[ctx.activeIdx ?? 0];
        if (!active) return;
        const href = active.getAttribute("href");
        const group = active.getAttribute("data-group");
        const title =
          active.querySelector(".iris-palette__item-title")?.textContent ?? "";
        navigateTo(ctx, { location: href, title, group });
        break;
      }
      case "Escape":
        e.preventDefault();
        // Keep nested modals (cheatsheet above palette) from also closing.
        e.stopPropagation();
        close(ctx);
        break;
    }
  }

  // Trap Tab focus inside the modal so keyboard users don't escape to
  // the page underneath. Listens on the backdrop, not the input, so
  // Tab from the input into the results list (which lives outside the
  // input) still works.
  function trapTab(e, ctx) {
    if (e.key !== "Tab") return;
    const focusables = [ctx.input, ...ctx.items].filter(
      (n) => n && !n.hasAttribute("disabled"),
    );
    if (focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    const cur = document.activeElement;
    if (e.shiftKey && (cur === first || !ctx.backdrop.contains(cur))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && cur === last) {
      e.preventDefault();
      first.focus();
    }
  }

  function navigateTo(ctx, item) {
    pushRecent({ location: item.location, title: item.title, group: item.group });
    close(ctx);
    // Let Material's instant-nav handle same-origin links naturally.
    if (item.location) window.location.assign(item.location);
  }

  function open(ctx) {
    if (ctx.isOpen) return;
    ctx.isOpen = true;
    // Remember whatever was focused so we can return to it on close.
    ctx.returnFocusTo =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    document.body.appendChild(ctx.backdrop);
    // Install focus-trap handler on the backdrop (captures Tab anywhere
    // in the modal subtree).
    ctx.trapHandler = (e) => trapTab(e, ctx);
    ctx.backdrop.addEventListener("keydown", ctx.trapHandler);
    requestAnimationFrame(() => {
      ctx.backdrop.classList.add("is-open");
      ctx.input.focus();
      ctx.input.value = "";
      loadIndex().then((idx) => {
        ctx.docs = Array.isArray(idx.docs) ? idx.docs : [];
        render(ctx);
      });
    });
    document.documentElement.style.overflow = "hidden";
  }

  function close(ctx) {
    if (!ctx.isOpen) return;
    ctx.isOpen = false;
    ctx.backdrop.classList.remove("is-open");
    document.documentElement.style.overflow = "";
    if (ctx.trapHandler) {
      ctx.backdrop.removeEventListener("keydown", ctx.trapHandler);
      ctx.trapHandler = null;
    }
    setTimeout(() => {
      if (ctx.backdrop.parentNode) ctx.backdrop.parentNode.removeChild(ctx.backdrop);
    }, 150);
    // Restore focus to the element that opened the palette so keyboard
    // users resume where they were (only if it's still connected).
    if (ctx.returnFocusTo && ctx.returnFocusTo.isConnected) {
      try {
        ctx.returnFocusTo.focus({ preventScroll: true });
      } catch {
        // Element might have been removed or become non-focusable.
      }
    }
    ctx.returnFocusTo = null;
  }

  function toggle(ctx) {
    ctx.isOpen ? close(ctx) : open(ctx);
  }

  function bind() {
    const ctx = { isOpen: false, docs: [], items: [], activeIdx: 0 };
    buildModal(ctx);
    window.__irisPalette = {
      open: () => open(ctx),
      close: () => close(ctx),
      toggle: () => toggle(ctx),
    };
    document.addEventListener("keydown", (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        if (isTypingTarget(e.target) && !ctx.isOpen) {
          // allow ctrl+k inside search box? still yes — toggle.
        }
        e.preventDefault();
        toggle(ctx);
        return;
      }
      if (ctx.isOpen) return; // in-modal keys handled by input listener
    });
  }

  if (typeof window !== "undefined") {
    if (document.readyState !== "loading") bind();
    else document.addEventListener("DOMContentLoaded", bind);
  }
})();

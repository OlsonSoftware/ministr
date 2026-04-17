// Thin scroll-progress bar fixed to the top of the viewport. Only shown
// on pages whose <article> is at least 2× viewport tall (deep architecture
// pages and long concepts). Uses requestAnimationFrame to throttle scroll
// handlers and respects prefers-reduced-motion by hiding the bar entirely.

(function () {
  "use strict";

  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    return;
  }

  let bar = null;
  let ticking = false;

  function ensureBar() {
    if (bar && bar.isConnected) return bar;
    bar = document.createElement("div");
    bar.className = "iris-reading-progress";
    bar.setAttribute("aria-hidden", "true");
    const fill = document.createElement("span");
    fill.className = "iris-reading-progress__fill";
    bar.appendChild(fill);
    document.body.appendChild(bar);
    return bar;
  }

  function removeBar() {
    if (bar && bar.parentNode) bar.parentNode.removeChild(bar);
    bar = null;
  }

  function getArticle() {
    return (
      document.querySelector("article.md-content__inner") ||
      document.querySelector("article") ||
      null
    );
  }

  function shouldShow(article) {
    if (!article) return false;
    const vh = window.innerHeight;
    return article.offsetHeight > vh * 2;
  }

  function update() {
    ticking = false;
    if (!bar) return;
    const article = getArticle();
    if (!article) return;
    const rect = article.getBoundingClientRect();
    const vh = window.innerHeight;
    const articleTop = rect.top + window.scrollY;
    const progress =
      (window.scrollY + vh - articleTop) / article.offsetHeight;
    const clamped = Math.max(0, Math.min(1, progress));
    const fill = bar.firstElementChild;
    if (fill) fill.style.transform = `scaleX(${clamped})`;
  }

  function onScroll() {
    if (!ticking) {
      ticking = true;
      requestAnimationFrame(update);
    }
  }

  function hydrate() {
    const article = getArticle();
    if (shouldShow(article)) {
      ensureBar();
      update();
    } else {
      removeBar();
    }
  }

  window.addEventListener("scroll", onScroll, { passive: true });
  window.addEventListener("resize", onScroll);

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

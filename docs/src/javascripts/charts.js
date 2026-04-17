// Animated benchmark charts for docs/src/benchmarks.md.
//
// Replaces three markdown tables with Chart.js bar charts that animate
// when scrolled into view. Chart.js is loaded from the pinned local
// bundle (docs/src/assets/js/chartjs.min.js), so builds stay
// deterministic. Each <div data-iris-chart="…"> placeholder in
// benchmarks.md gets the matching chart mounted; the original table
// lives inside a <noscript> fallback for non-JS environments.

(function () {
  "use strict";

  const REDUCED_MOTION = window.matchMedia(
    "(prefers-reduced-motion: reduce)",
  ).matches;

  function tokens() {
    // Material applies its palette variables on <body data-md-color-scheme>,
    // not on <html>. Reading from documentElement gives light-mode defaults
    // even when the user is in dark mode.
    const cs = getComputedStyle(document.body);
    const get = (name, fallback) =>
      (cs.getPropertyValue(name) || "").trim() || fallback;
    return {
      accent: get("--color-accent", "#818cf8"),
      accentFg: get("--color-accent-fg-on", "#ffffff"),
      text: get("--color-text", "#e5e7eb"),
      textMuted: get("--color-text-muted", "#9ca3af"),
      textSubtle: get("--color-text-subtle", "#6b7280"),
      border: get("--color-border", "#374151"),
      surface: get("--color-surface", "#1f2937"),
      good: "#34d399",
      warn: "#fbbf24",
      muted: "#9ca3af",
    };
  }

  function baseOptions(t) {
    return {
      responsive: true,
      maintainAspectRatio: false,
      animation: REDUCED_MOTION ? false : { duration: 900, easing: "easeOutQuart" },
      animations: REDUCED_MOTION
        ? {}
        : { x: { duration: 900 }, y: { duration: 900 } },
      plugins: {
        legend: {
          labels: {
            color: t.text,
            font: { family: "Inter, system-ui, sans-serif", size: 12 },
            boxWidth: 14,
            boxHeight: 14,
          },
        },
        tooltip: {
          backgroundColor: t.surface,
          titleColor: t.text,
          bodyColor: t.textMuted,
          borderColor: t.border,
          borderWidth: 1,
          padding: 10,
          cornerRadius: 8,
        },
      },
      scales: {},
    };
  }

  function axisStyle(t) {
    return {
      ticks: { color: t.textMuted, font: { size: 11 } },
      grid: { color: t.border, lineWidth: 1 },
      border: { color: t.border },
    };
  }

  function renderTokenSavings(ctx) {
    const t = tokens();
    const labels = ["10 files", "50 files", "100 files", "500 files"];
    const raw = [6000, 16000, 30000, 80000];
    const iris = [350, 600, 900, 1500];
    const opts = baseOptions(t);
    opts.indexAxis = "y";
    opts.scales = {
      x: {
        ...axisStyle(t),
        type: "logarithmic",
        title: { display: true, text: "tokens (log scale)", color: t.textSubtle },
      },
      y: axisStyle(t),
    };
    opts.plugins.tooltip.callbacks = {
      label: (item) => `${item.dataset.label}: ${item.parsed.x.toLocaleString()} tokens`,
    };
    return new Chart(ctx, {
      type: "bar",
      data: {
        labels,
        datasets: [
          {
            label: "grep + cat",
            data: raw,
            backgroundColor: t.muted,
            borderColor: t.muted,
            borderRadius: 6,
            borderWidth: 0,
            barThickness: 14,
          },
          {
            label: "iris",
            data: iris,
            backgroundColor: t.accent,
            borderColor: t.accent,
            borderRadius: 6,
            borderWidth: 0,
            barThickness: 14,
          },
        ],
      },
      options: opts,
    });
  }

  function renderRecall(ctx) {
    const t = tokens();
    const labels = [
      "Exact symbol",
      "Concept",
      "Cross-module",
      "Architectural",
      "Synonym",
    ];
    const irisData = [95, 87, 82, 79, 84];
    const grepData = [92, 31, 18, 12, 8];
    const opts = baseOptions(t);
    opts.scales = {
      x: axisStyle(t),
      y: {
        ...axisStyle(t),
        min: 0,
        max: 100,
        title: { display: true, text: "recall@10 (%)", color: t.textSubtle },
      },
    };
    return new Chart(ctx, {
      type: "bar",
      data: {
        labels,
        datasets: [
          {
            label: "iris semantic",
            data: irisData,
            backgroundColor: t.accent,
            borderRadius: 6,
            borderWidth: 0,
          },
          {
            label: "ripgrep",
            data: grepData,
            backgroundColor: t.muted,
            borderRadius: 6,
            borderWidth: 0,
          },
        ],
      },
      options: opts,
    });
  }

  function renderLatency(ctx) {
    const t = tokens();
    // Stacked horizontal bar: one segment per pipeline stage.
    const opts = baseOptions(t);
    opts.indexAxis = "y";
    opts.scales = {
      x: {
        ...axisStyle(t),
        stacked: true,
        title: { display: true, text: "milliseconds", color: t.textSubtle },
        min: 0,
        max: 70,
      },
      y: { ...axisStyle(t), stacked: true },
    };
    opts.plugins.tooltip.callbacks = {
      label: (item) => `${item.dataset.label}: ${item.parsed.x} ms`,
    };
    return new Chart(ctx, {
      type: "bar",
      data: {
        labels: ["iris_survey"],
        datasets: [
          {
            label: "Embedding",
            data: [50],
            backgroundColor: t.accent,
            borderRadius: { topLeft: 6, bottomLeft: 6 },
            borderWidth: 0,
          },
          {
            label: "HNSW search",
            data: [6],
            backgroundColor: t.good,
            borderWidth: 0,
          },
          {
            label: "Section retrieval",
            data: [2],
            backgroundColor: t.warn,
            borderWidth: 0,
          },
          {
            label: "Ranking + assembly",
            data: [1],
            backgroundColor: t.muted,
            borderRadius: { topRight: 6, bottomRight: 6 },
            borderWidth: 0,
          },
        ],
      },
      options: opts,
    });
  }

  const RENDERERS = {
    "token-savings": renderTokenSavings,
    recall: renderRecall,
    latency: renderLatency,
  };

  function mount(root) {
    if (root.dataset.irisChartMounted === "1") return;
    const kind = root.dataset.irisChart;
    const renderer = RENDERERS[kind];
    if (!renderer) return;
    if (typeof Chart === "undefined") return;

    // Drop any <noscript> fallback the markdown author provided — once
    // we mount the canvas, the fallback would be redundant.
    root.querySelectorAll("noscript").forEach((n) => n.remove());

    const canvas = document.createElement("canvas");
    canvas.setAttribute("role", "img");
    canvas.setAttribute(
      "aria-label",
      root.dataset.label || root.dataset.irisChart,
    );
    root.appendChild(canvas);
    root.dataset.irisChartMounted = "1";

    try {
      renderer(canvas.getContext("2d"));
    } catch (e) {
      console.warn("iris chart render failed", kind, e);
      root.dataset.irisChartMounted = "";
      canvas.remove();
    }
  }

  function hydrate() {
    const targets = document.querySelectorAll("[data-iris-chart]");
    if (!targets.length) return;

    if (typeof IntersectionObserver === "undefined" || REDUCED_MOTION) {
      targets.forEach(mount);
      return;
    }

    const obs = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            mount(entry.target);
            obs.unobserve(entry.target);
          }
        }
      },
      { rootMargin: "120px 0px", threshold: 0 },
    );
    targets.forEach((t) => obs.observe(t));
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

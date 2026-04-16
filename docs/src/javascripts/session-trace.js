// Animated live session trace for the iris landing page.
//
// Replaces the static code block that used to list a typical iris session
// with a JS-driven typewriter + budget gauge + cache-hit / prefetch
// indicators. The replay loops every ~30 seconds, pauses on hover, and
// toggles with the space key while focused. Respects prefers-reduced-motion
// by rendering the final state immediately with no animation.

(function () {
  "use strict";

  // Each step is one logical tool call. `line` is the command text, `meta`
  // is the sub-line iris prints back. `budget` is the budget % AFTER this
  // step completes (gauge animates toward it). `pause` is the dwell time
  // after the line finishes typing before the next step begins. `tag` is
  // an optional pill: cache-hit | prefetch | pressure | evict.
  const SCRIPT = [
    {
      line: 'iris_survey("authentication middleware")',
      meta: "ranked 5 results · prefetch: warming src/auth.rs#logout",
      budget: 3,
      pause: 900,
      tag: "prefetch",
    },
    {
      line: 'iris_read("src/auth.rs#login")',
      meta: "420 tokens · prefetch: warming validate_token (structural)",
      budget: 5,
      pause: 900,
      tag: "prefetch",
    },
    {
      line: 'iris_read("src/auth.rs#logout")',
      meta: "CACHE HIT — delivered from prefetch · 0 ms",
      budget: 7,
      pause: 1000,
      tag: "cache-hit",
    },
    {
      line: 'iris_symbols(kind="function", query="validate")',
      meta: "8 symbols found",
      budget: 8,
      pause: 900,
    },
    {
      line: "… many reads later …",
      meta: null,
      budget: 60,
      pause: 700,
      tag: "ellipsis",
    },
    {
      line: 'iris_survey("rate limiting")',
      meta:
        "results at CLAIM resolution · pressure: ELEVATED · " +
        "eviction_recommendations: [src/setup.rs#prerequisites, docs/intro.md]",
      budget: 82,
      pause: 1100,
      tag: "pressure",
    },
    {
      line: 'iris_evicted(["src/setup.rs#prerequisites"])',
      meta: "session shadow updated",
      budget: 76,
      pause: 1400,
      tag: "evict",
    },
  ];

  const REDUCED_MOTION = window.matchMedia(
    "(prefers-reduced-motion: reduce)",
  ).matches;

  const CHAR_DELAY = 12; // ms per typed char
  const LOOP_DELAY = 3000; // pause before the replay starts over

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }

  function sleep(ms, controller) {
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => {
        controller.removeListener(onAbort);
        resolve();
      }, ms);
      function onAbort() {
        clearTimeout(t);
        reject(new Error("aborted"));
      }
      controller.addListener(onAbort);
    });
  }

  // AbortController-style signal that also supports pause/resume. We don't
  // actually need cancellation — just a way to stop the current run when
  // the component is rebuilt (Material's instant navigation) or paused.
  function makeController() {
    const listeners = new Set();
    let aborted = false;
    return {
      abort() {
        aborted = true;
        listeners.forEach((l) => l());
      },
      addListener(l) {
        listeners.add(l);
      },
      removeListener(l) {
        listeners.delete(l);
      },
      get aborted() {
        return aborted;
      },
    };
  }

  function build(root) {
    root.innerHTML = "";
    root.classList.add("iris-trace-live");

    const chrome = el("div", "iris-trace-live__chrome");
    const dots = el("div", "iris-trace-live__dots");
    dots.appendChild(el("span", "iris-trace-live__dot iris-trace-live__dot--r"));
    dots.appendChild(el("span", "iris-trace-live__dot iris-trace-live__dot--y"));
    dots.appendChild(el("span", "iris-trace-live__dot iris-trace-live__dot--g"));
    chrome.appendChild(dots);
    chrome.appendChild(el("span", "iris-trace-live__title", "iris session · live"));

    const status = el("div", "iris-trace-live__status");
    const turn = el("span", "iris-trace-live__turn", "turn 0");
    const prefetch = el("span", "iris-trace-live__prefetch");
    prefetch.innerHTML =
      '<span class="iris-trace-live__led"></span>PREFETCH';
    const gauge = el("div", "iris-trace-live__gauge");
    const fill = el("span", "iris-trace-live__gauge-fill");
    const pct = el("span", "iris-trace-live__gauge-pct", "0%");
    gauge.appendChild(fill);
    gauge.appendChild(pct);
    status.appendChild(turn);
    status.appendChild(prefetch);
    status.appendChild(gauge);
    chrome.appendChild(status);

    root.appendChild(chrome);

    const body = el("div", "iris-trace-live__body");
    const lines = el("div", "iris-trace-live__lines");
    body.appendChild(lines);
    root.appendChild(body);

    return { lines, fill, pct, turn, prefetch };
  }

  async function typeText(node, text, controller) {
    if (REDUCED_MOTION) {
      node.textContent = text;
      return;
    }
    for (let i = 0; i < text.length; i++) {
      if (controller.aborted) return;
      node.textContent = text.slice(0, i + 1);
      await sleep(CHAR_DELAY, controller);
    }
  }

  async function runOnce(ctx, controller) {
    const { lines, fill, pct, turn, prefetch } = ctx;
    lines.innerHTML = "";
    fill.style.width = "0%";
    pct.textContent = "0%";
    turn.textContent = "turn 0";
    prefetch.classList.remove("is-on");

    for (let i = 0; i < SCRIPT.length; i++) {
      if (controller.aborted) return;
      const step = SCRIPT[i];

      const row = el("div", "iris-trace-live__row");
      if (step.tag) row.dataset.tag = step.tag;

      const marker = el("span", "iris-trace-live__marker", "→");
      const cmd = el("span", "iris-trace-live__cmd");
      row.appendChild(marker);
      row.appendChild(cmd);
      lines.appendChild(row);

      if (step.tag === "prefetch") prefetch.classList.add("is-on");
      else prefetch.classList.remove("is-on");

      turn.textContent = `turn ${i + 1}`;

      await typeText(cmd, step.line, controller);
      if (controller.aborted) return;

      if (step.meta) {
        const meta = el("span", "iris-trace-live__meta");
        row.appendChild(meta);
        await typeText(meta, step.meta, controller);
        if (controller.aborted) return;
      }

      if (step.tag === "cache-hit") {
        row.appendChild(el("span", "iris-trace-live__badge iris-trace-live__badge--hit", "HIT"));
      } else if (step.tag === "evict") {
        row.appendChild(el("span", "iris-trace-live__badge iris-trace-live__badge--evict", "EVICT"));
      } else if (step.tag === "pressure") {
        row.appendChild(el("span", "iris-trace-live__badge iris-trace-live__badge--warn", "PRESSURE"));
      }

      // Animate budget gauge toward the new target.
      fill.style.width = step.budget + "%";
      pct.textContent = step.budget + "%";

      // Auto-scroll latest row into view without hijacking page scroll.
      lines.scrollTop = lines.scrollHeight;

      if (!REDUCED_MOTION) {
        await sleep(step.pause, controller);
        if (controller.aborted) return;
      }
    }
  }

  async function loop(ctx, controller, getPaused) {
    while (!controller.aborted) {
      await runOnce(ctx, controller);
      if (controller.aborted) return;
      // Hold final state before replaying.
      try {
        const start = Date.now();
        while (Date.now() - start < LOOP_DELAY) {
          if (controller.aborted) return;
          if (getPaused()) {
            await sleep(200, controller);
            continue;
          }
          await sleep(100, controller);
        }
      } catch (_) {
        return;
      }
      while (getPaused()) {
        if (controller.aborted) return;
        await sleep(200, controller);
      }
    }
  }

  function mount(root) {
    const ctx = build(root);
    const controller = makeController();
    let paused = false;

    const onEnter = () => {
      paused = true;
      root.classList.add("is-paused");
    };
    const onLeave = () => {
      paused = false;
      root.classList.remove("is-paused");
    };
    const onKey = (e) => {
      if (e.code === "Space" && document.activeElement === root) {
        e.preventDefault();
        paused = !paused;
        root.classList.toggle("is-paused", paused);
      }
    };

    root.addEventListener("mouseenter", onEnter);
    root.addEventListener("mouseleave", onLeave);
    root.tabIndex = 0;
    root.setAttribute(
      "aria-label",
      "Live iris session trace. Space toggles playback.",
    );
    root.addEventListener("keydown", onKey);

    loop(ctx, controller, () => paused).catch(() => {});

    return () => {
      controller.abort();
      root.removeEventListener("mouseenter", onEnter);
      root.removeEventListener("mouseleave", onLeave);
      root.removeEventListener("keydown", onKey);
    };
  }

  const teardowns = new WeakMap();

  function hydrate() {
    document.querySelectorAll("[data-iris-trace]").forEach((root) => {
      if (teardowns.has(root)) return;
      const teardown = mount(root);
      teardowns.set(root, teardown);
    });
  }

  function dehydrate() {
    teardowns.forEach((teardown) => teardown());
  }

  // Material's instant navigation swaps page content without a full reload.
  // If Material exposes `document$` (an RxJS stream of page loads) hook into
  // it; otherwise fall back to DOMContentLoaded.
  if (typeof window !== "undefined") {
    if (window.document$ && typeof window.document$.subscribe === "function") {
      window.document$.subscribe(hydrate);
    } else if (document.readyState !== "loading") {
      hydrate();
    } else {
      document.addEventListener("DOMContentLoaded", hydrate);
    }

    // Clean up when leaving the page (instant nav handles this by
    // rebuilding document; for hard nav the GC will take care of it).
    window.addEventListener("beforeunload", dehydrate);
  }
})();

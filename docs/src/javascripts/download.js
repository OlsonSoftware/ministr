// Platform-aware + release-aware download page enhancements.
//
// On the /download/ page this script:
//   • sniffs os/arch from navigator.userAgentData (Chromium) or the UA
//     string, and rewrites the primary CTA to the matching release
//     artifact (href, label, arch tag, size, min-OS caption).
//   • fetches the latest GitHub release via the public API to replace
//     the hard-coded version + date with live values and link the
//     "What's new" button to the matching release notes.
//   • opens a DNS + TLS preconnect to the release host on hover, so
//     the actual download feels instantaneous.
//
// Everything fails silently — if the fetch is blocked or detection
// fails, the page still works from the static HTML fallback. Copy-to-
// clipboard is handled by Material's built-in content.code.copy feature;
// no custom copy button here.

(function () {
  "use strict";

  /* ------------------------------------------------------------------ *
   * 1. Platform-aware primary CTA swap                                  *
   * ------------------------------------------------------------------ */

  const root = document.querySelector("[data-iris-download]");
  if (root) {
    enhanceDownload(root);
    installStickyBar(root);
  }

  /* ------------------------------------------------------------------ *
   * Recent-releases changelog strip                                    *
   * ------------------------------------------------------------------ */

  const changelog = document.querySelector("[data-iris-changelog]");
  if (changelog) {
    installChangelogStrip(changelog);
  }

  // --------------------------------------------------------------------
  // Primary download card
  // --------------------------------------------------------------------

  function enhanceDownload(root) {
    const version = root.dataset.version || "0.1.0";
    const base = root.dataset.releaseBase || "";
    const repo = root.dataset.repo || "";
    const primaryLink = root.querySelector("[data-primary-link]");
    const primaryLabel = root.querySelector("[data-primary-label]");
    const primaryBadge = root.querySelector("[data-primary-badge]");
    const sizeEl = root.querySelector("[data-iris-size]");
    const archEl = root.querySelector("[data-iris-arch]");
    const minOsEl = root.querySelector("[data-iris-minos]");
    const captionEl = root.querySelector("[data-iris-caption]");
    const releaseEl = root.querySelector("[data-iris-release]");
    const versionEl = root.querySelector("[data-iris-version]");
    const reldateEl = root.querySelector("[data-iris-reldate]");
    const notesEl = root.querySelector("[data-iris-notes]");
    const primaryCard = root.querySelector(".iris-download__primary");

    if (!primaryLink || !primaryLabel) return;

    detect()
      .then((platform) => applyPlatform(platform))
      .catch(() => applyPlatform({ os: "macos", arch: "arm64" }));

    if (repo) {
      fetchLatestRelease(repo)
        .then((rel) => applyRelease(rel))
        .catch(() => {
          // Keep the static defaults visible on fetch failure.
        });
    }

    prefetchOnHover(primaryLink);
    installPostClick(primaryLink);

    // ----- Platform detection ---------------------------------------

    function detect() {
      const ua = navigator.userAgent || "";
      const uaData = navigator.userAgentData;

      let os = "macos";
      if (uaData && uaData.platform) {
        const p = uaData.platform.toLowerCase();
        if (p.includes("win")) os = "windows";
        else if (p.includes("mac")) os = "macos";
        else if (p.includes("linux") || p.includes("android")) os = "linux";
      } else {
        if (/Windows/i.test(ua)) os = "windows";
        else if (/Mac|Macintosh/i.test(ua)) os = "macos";
        else if (/Linux|CrOS|Android/i.test(ua)) os = "linux";
      }

      if (uaData && typeof uaData.getHighEntropyValues === "function") {
        return uaData
          .getHighEntropyValues([
            "architecture",
            "bitness",
            "platformVersion",
          ])
          .then((hints) => ({
            os,
            arch: archFromHints(os, hints),
            platformVersion: hints && hints.platformVersion,
          }))
          .catch(() => ({ os, arch: archFallback(os, ua) }));
      }
      return Promise.resolve({ os, arch: archFallback(os, ua) });
    }

    function archFromHints(os, hints) {
      if (!hints || !hints.architecture) {
        return os === "macos" ? "arm64" : "x64";
      }
      if (hints.architecture === "x86") return "x64";
      if (hints.architecture === "arm") return "arm64";
      return os === "macos" ? "arm64" : "x64";
    }

    function archFallback(os, ua) {
      if (os === "macos") return /Intel/i.test(ua) ? "x64" : "arm64";
      if (/aarch64|arm64/i.test(ua)) return "arm64";
      return "x64";
    }

    function artifactFor(os, arch) {
      if (os === "macos") {
        return {
          file: `iris-${version}.pkg`,
          label:
            arch === "arm64"
              ? "Download for macOS — Apple Silicon"
              : "Download for macOS — Intel",
          arch: arch === "arm64" ? "Apple Silicon" : "Intel Mac",
          kind: "Signed + notarized .pkg (universal)",
          size: "~48 MB",
          minOs: "macOS 13 Ventura or newer",
          badge: "Recommended for your Mac",
        };
      }
      if (os === "windows") {
        return {
          file: `iris_${version}_x64-setup.exe`,
          label: "Download for Windows",
          arch: "x64",
          kind: "NSIS installer · desktop app + CLI",
          size: "~42 MB",
          minOs: "Windows 10 1809 or newer",
          badge: "Recommended for Windows",
        };
      }
      if (os === "linux") {
        return {
          file: `iris_${version}_amd64.AppImage`,
          label: "Download for Linux",
          arch: arch === "arm64" ? "arm64" : "x64",
          kind: "AppImage · portable, no install",
          size: "~52 MB",
          minOs: "glibc 2.31 or newer",
          badge: "Recommended for Linux",
        };
      }
      return null;
    }

    function applyPlatform({ os, arch, platformVersion }) {
      const art = artifactFor(os, arch);
      if (!art) {
        finishSkeleton(false);
        return;
      }

      primaryLink.href = `${base}/${art.file}`;
      primaryLabel.textContent = art.label;
      if (archEl) archEl.textContent = art.arch;
      if (sizeEl) sizeEl.textContent = art.size;
      if (minOsEl) minOsEl.textContent = art.minOs;
      if (primaryBadge) primaryBadge.textContent = art.badge;

      // Populate the platform caption.
      if (captionEl) {
        const osLabel = osDisplay(os);
        const archLabel = art.arch;
        const ver = platformVersion ? ` · ${osLabel} ${platformVersion}` : "";
        captionEl.innerHTML =
          `<span class="iris-download__caption-label">Detected:</span> ` +
          `${osLabel} · ${archLabel}${ver}. ` +
          `<a href="#other-platforms" class="iris-download__caption-link">Not right? See all downloads →</a>`;
      }

      root.setAttribute("data-detected-os", os);
      root.setAttribute("data-detected-arch", arch);

      // OS-aware install-flow stepper: rewrite each step's label to
      // reflect the visitor's platform. Each <li> carries data-mac,
      // data-win, data-lin; we pick the matching one if set.
      swapInstallFlow(os);

      // macOS version gate: warn if detected platformVersion < 13.0.
      if (os === "macos" && platformVersion) {
        const major = parseInt(platformVersion.split(".")[0], 10);
        if (!Number.isNaN(major) && major < 13) {
          showMinOsWarning(
            `Your Mac reports macOS ${platformVersion}, but iris requires macOS 13 Ventura or newer. The installer will refuse to run.`
          );
        }
      }

      finishSkeleton(true);
    }

    function swapInstallFlow(os) {
      const flow = document.querySelector("[data-iris-flow]");
      if (!flow) return;
      const key = os === "windows" ? "win" : os === "linux" ? "lin" : "mac";
      flow.querySelectorAll(".iris-install-flow__step").forEach((step) => {
        const variant = step.dataset[key];
        if (!variant) return;
        const label = step.querySelector(".iris-install-flow__label");
        if (label) label.innerHTML = variant;
      });
      flow.setAttribute("data-detected-os", os);
    }

    function showMinOsWarning(msg) {
      if (!primaryCard) return;
      let w = primaryCard.querySelector(".iris-download__warning");
      if (!w) {
        w = document.createElement("div");
        w.className = "iris-download__warning";
        w.setAttribute("role", "alert");
        const ctaAnchor = primaryCard.querySelector(".iris-download__caption");
        if (ctaAnchor) ctaAnchor.after(w);
        else primaryCard.appendChild(w);
      }
      w.innerHTML = `<span>${msg}</span>`;
      w.removeAttribute("hidden");
    }

    function osDisplay(os) {
      if (os === "macos") return "macOS";
      if (os === "windows") return "Windows";
      if (os === "linux") return "Linux";
      return os;
    }

    function finishSkeleton(ok) {
      if (primaryCard) {
        primaryCard.setAttribute("data-state", ok ? "ready" : "fallback");
      }
    }

    // ----- GitHub release API ---------------------------------------

    async function fetchLatestRelease(repo) {
      const r = await fetch(`https://api.github.com/repos/${repo}/releases/latest`, {
        headers: { Accept: "application/vnd.github+json" },
        mode: "cors",
      });
      if (!r.ok) throw new Error(`GitHub API ${r.status}`);
      const body = await r.json();
      return {
        tag: body.tag_name || "",
        htmlUrl: body.html_url || "",
        publishedAt: body.published_at || "",
        assets: body.assets || [],
        body: body.body || "",
      };
    }

    function applyRelease(rel) {
      if (!rel || !releaseEl) return;

      if (rel.tag && versionEl) {
        versionEl.textContent = rel.tag.startsWith("v") ? rel.tag : `v${rel.tag}`;
      }
      if (rel.publishedAt && reldateEl) {
        reldateEl.textContent = humanDate(rel.publishedAt);
      }
      if (rel.htmlUrl && notesEl) {
        notesEl.href = rel.htmlUrl;
      }

      // Swap primary artifact size for the real thing if we can match it
      // against a release asset.
      if (rel.assets && rel.assets.length) {
        const url = new URL(primaryLink.href);
        const fname = url.pathname.split("/").pop();
        const match = rel.assets.find((a) => a.name === fname);
        if (match && sizeEl) {
          sizeEl.textContent = humanSize(match.size);
        }
      }

      // Two-line preview of the release body — first non-empty line that
      // isn't a markdown heading, stripped of common prefix chars.
      const previewEl = root.querySelector("[data-iris-preview]");
      if (previewEl && rel.body) {
        const preview = extractReleasePreview(rel.body);
        if (preview) {
          previewEl.textContent = preview;
          previewEl.removeAttribute("hidden");
        }
      }

      // Social proof: sum download counts across all release assets so
      // visitors see real adoption numbers instead of a static claim.
      const proofEl = root.querySelector("[data-iris-proof]");
      const proofText = root.querySelector("[data-iris-proof-text]");
      if (proofEl && proofText && rel.assets && rel.assets.length) {
        const total = rel.assets.reduce(
          (n, a) => n + (a.download_count || 0),
          0
        );
        if (total >= 50) {
          proofText.textContent = `${humanCount(total)} downloads of this release`;
          proofEl.removeAttribute("hidden");
        }
      }

      releaseEl.removeAttribute("hidden");
    }

    function humanCount(n) {
      if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`;
      if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
      return `${n}`;
    }

    function extractReleasePreview(body) {
      const lines = body.split(/\r?\n/).map((l) => l.trim());
      // Find the first line that isn't a heading, HR, or blank.
      let text = "";
      for (const line of lines) {
        if (!line) continue;
        if (/^#{1,6}\s/.test(line)) continue;
        if (/^[-*_]{3,}\s*$/.test(line)) continue;
        text = line.replace(/^[-*+]\s+/, "").replace(/^\d+\.\s+/, "");
        // Strip inline code ticks and bold/italic markers for readability.
        text = text
          .replace(/`([^`]+)`/g, "$1")
          .replace(/\*\*([^*]+)\*\*/g, "$1")
          .replace(/\*([^*]+)\*/g, "$1")
          .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1");
        break;
      }
      if (!text) return "";
      if (text.length > 200) text = text.slice(0, 197) + "…";
      return text;
    }

    function humanDate(iso) {
      try {
        const d = new Date(iso);
        const now = new Date();
        const diffDays = Math.round((now - d) / 86400000);
        if (diffDays <= 0) return "released today";
        if (diffDays === 1) return "released yesterday";
        if (diffDays < 14) return `released ${diffDays} days ago`;
        return `released ${d.toLocaleDateString(undefined, {
          year: "numeric",
          month: "short",
          day: "numeric",
        })}`;
      } catch {
        return "released recently";
      }
    }

    function humanSize(bytes) {
      if (!bytes) return "";
      const mb = bytes / (1024 * 1024);
      if (mb >= 10) return `${Math.round(mb)} MB`;
      return `${mb.toFixed(1)} MB`;
    }

    // ----- Post-click "downloading…" + scroll ------------------------

    function installPostClick(link) {
      const post = root.querySelector("[data-iris-postclick]");
      if (!post) return;
      const fileEl = post.querySelector("[data-postclick-file]");
      const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

      link.addEventListener("click", () => {
        // Let the browser keep the href resolution — we're not preventing
        // default. This is purely an advisory + nav helper.
        if (fileEl) {
          try {
            fileEl.textContent =
              new URL(link.href).pathname.split("/").pop() || "iris.pkg";
          } catch {
            fileEl.textContent = "iris.pkg";
          }
        }
        post.removeAttribute("hidden");
        requestAnimationFrame(() =>
          post.setAttribute("data-visible", "1")
        );

        // Smooth-scroll the after-install section into view so the user
        // sees their next move while the download is running.
        const next = document.getElementById("wire-it-into-your-agent");
        if (next) {
          window.setTimeout(
            () => {
              next.scrollIntoView({
                behavior: reduced ? "auto" : "smooth",
                block: "start",
              });
            },
            reduced ? 100 : 900
          );
        }
      });
    }

    // ----- Hover prefetch -------------------------------------------

    function prefetchOnHover(link) {
      let timer = null;
      let prefetched = false;
      link.addEventListener("pointerenter", () => {
        if (prefetched || !link.href) return;
        timer = window.setTimeout(() => {
          // `preconnect` warms DNS + TLS to the CDN host without fetching
          // the (potentially 50 MB) artifact body. Cheap, zero waste on
          // visitors who hover but don't click.
          const pre = document.createElement("link");
          pre.rel = "preconnect";
          try {
            pre.href = new URL(link.href).origin;
          } catch {
            return;
          }
          pre.crossOrigin = "anonymous";
          document.head.appendChild(pre);
          prefetched = true;
        }, 600);
      });
      link.addEventListener("pointerleave", () => {
        if (timer !== null) window.clearTimeout(timer);
        timer = null;
      });
    }
  }

  // --------------------------------------------------------------------
  // Sticky mini-download bar
  // --------------------------------------------------------------------

  function installStickyBar(root) {
    const sticky = document.querySelector("[data-iris-download-sticky]");
    if (!sticky) return;

    const primaryLink = root.querySelector("[data-primary-link]");
    const primaryLabel = root.querySelector("[data-primary-label]");
    const archEl = root.querySelector("[data-iris-arch]");
    const sizeEl = root.querySelector("[data-iris-size]");
    const mirrorLink = sticky.querySelector("[data-primary-mirror]");
    const mirrorLabel = sticky.querySelector("[data-mirror-label]");
    const mirrorSub = sticky.querySelector("[data-mirror-sub]");
    const dismiss = sticky.querySelector("[data-sticky-dismiss]");

    if (!primaryLink || !mirrorLink) return;

    // Respect an earlier dismiss within the same session.
    const dismissKey = "iris:download-sticky:dismissed";
    if (sessionStorage.getItem(dismissKey) === "1") return;

    // Keep the mirror link in sync with the primary, both on first
    // platform-detect and on subsequent DOM mutations. A MutationObserver
    // on the primary link's href is the least-coupled option.
    const sync = () => {
      mirrorLink.href = primaryLink.href;
      if (mirrorLabel && primaryLabel) {
        // The primary label reads "Download for macOS — Apple Silicon";
        // the sticky version is tighter: just "Download iris".
        mirrorLabel.textContent = "Download iris";
      }
      if (mirrorSub) {
        const parts = [];
        if (archEl && archEl.textContent) parts.push(archEl.textContent);
        if (sizeEl && sizeEl.textContent) parts.push(sizeEl.textContent);
        mirrorSub.textContent = parts.join(" · ") || "";
      }
    };

    sync();
    const mo = new MutationObserver(sync);
    mo.observe(primaryLink, { attributes: true, attributeFilter: ["href"] });
    if (archEl) mo.observe(archEl, { childList: true, characterData: true, subtree: true });
    if (sizeEl) mo.observe(sizeEl, { childList: true, characterData: true, subtree: true });

    // Show the sticky when the primary CTA scrolls out of view.
    const io = new IntersectionObserver(
      ([entry]) => {
        if (!entry) return;
        const out = !entry.isIntersecting;
        if (out) {
          sticky.removeAttribute("hidden");
          requestAnimationFrame(() => sticky.setAttribute("data-visible", "1"));
        } else {
          sticky.removeAttribute("data-visible");
          // Defer the hidden attribute until the transition completes.
          window.setTimeout(() => {
            if (!sticky.hasAttribute("data-visible")) {
              sticky.setAttribute("hidden", "");
            }
          }, 260);
        }
      },
      { rootMargin: "0px 0px -40% 0px", threshold: 0 }
    );
    io.observe(primaryLink);

    if (dismiss) {
      dismiss.addEventListener("click", () => {
        sessionStorage.setItem(dismissKey, "1");
        sticky.removeAttribute("data-visible");
        window.setTimeout(() => sticky.setAttribute("hidden", ""), 260);
        io.disconnect();
        mo.disconnect();
      });
    }
  }

  // --------------------------------------------------------------------
  // Recent-releases changelog strip
  // --------------------------------------------------------------------

  function installChangelogStrip(root) {
    const repo = root.dataset.repo;
    if (!repo) return;

    // The skeleton row is server-rendered `hidden` so no-JS visitors
    // never see "Loading recent releases…" stuck forever. Reveal it
    // now that the script is running.
    root
      .querySelectorAll(".iris-changelog__item--skeleton")
      .forEach((el) => el.removeAttribute("hidden"));

    // If the fetch stalls or fails, the skeleton row would stick around
    // reading "Loading recent releases…" indefinitely. Give up after
    // 6 seconds, drop the skeleton, and leave only the static "See full
    // changelog →" link.
    const giveUp = window.setTimeout(dropSkeleton, 6000);

    fetch(`https://api.github.com/repos/${repo}/releases?per_page=3`, {
      headers: { Accept: "application/vnd.github+json" },
      mode: "cors",
    })
      .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
      .then((releases) => {
        window.clearTimeout(giveUp);
        if (Array.isArray(releases) && releases.length > 0) {
          render(releases);
        } else {
          dropSkeleton();
        }
      })
      .catch(() => {
        window.clearTimeout(giveUp);
        dropSkeleton();
      });

    function dropSkeleton() {
      root
        .querySelectorAll(".iris-changelog__item--skeleton")
        .forEach((el) => el.remove());
      root.setAttribute("data-state", "fallback");
    }

    function render(releases) {
      // Drop the skeleton + fallback-only state before inserting real rows.
      root
        .querySelectorAll(".iris-changelog__item--skeleton")
        .forEach((el) => el.remove());

      const frag = document.createDocumentFragment();
      for (const rel of releases.slice(0, 3)) {
        frag.appendChild(renderItem(rel));
      }

      // Insert real items before the static "full changelog" link so it
      // stays at the bottom.
      const fallback = root.querySelector(".iris-changelog__fallback");
      if (fallback) {
        root.insertBefore(frag, fallback);
      } else {
        root.appendChild(frag);
      }
      root.setAttribute("data-state", "ready");
    }

    function renderItem(rel) {
      const a = document.createElement("a");
      a.className = "iris-changelog__item";
      a.href = rel.html_url || `https://github.com/${repo}/releases`;
      a.setAttribute("rel", "noopener");

      const tag = document.createElement("span");
      tag.className = "iris-changelog__tag";
      tag.textContent = (rel.tag_name || "v?").replace(/^v?/, "v");

      const date = document.createElement("span");
      date.className = "iris-changelog__date";
      date.textContent = rel.published_at
        ? shortDate(rel.published_at)
        : "";

      const body = document.createElement("span");
      body.className = "iris-changelog__body";
      body.textContent = extractFirstLine(rel.body) || rel.name || "Release notes";

      a.append(tag, date, body);
      return a;
    }

    function shortDate(iso) {
      try {
        return new Date(iso).toLocaleDateString(undefined, {
          month: "short",
          day: "numeric",
          year: "numeric",
        });
      } catch {
        return "";
      }
    }

    function extractFirstLine(body) {
      if (!body) return "";
      const lines = body.split(/\r?\n/).map((l) => l.trim());
      for (const line of lines) {
        if (!line) continue;
        if (/^#{1,6}\s/.test(line)) continue;
        if (/^[-*_]{3,}\s*$/.test(line)) continue;
        let text = line.replace(/^[-*+]\s+/, "").replace(/^\d+\.\s+/, "");
        text = text
          .replace(/`([^`]+)`/g, "$1")
          .replace(/\*\*([^*]+)\*\*/g, "$1")
          .replace(/\*([^*]+)\*/g, "$1")
          .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1");
        if (text.length > 140) text = text.slice(0, 137) + "…";
        return text;
      }
      return "";
    }
  }

})();

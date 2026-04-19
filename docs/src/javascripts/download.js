// Platform-aware + release-aware download page enhancements.
//
// On the /download/ page this script:
//   • sniffs os/arch from navigator.userAgentData (Chromium) or the UA
//     string, and rewrites the primary CTA to the matching release
//     artifact (href, label, arch tag, size, min-OS caption).
//   • fetches the latest GitHub release via the public API to replace
//     the hard-coded version + date with live values and link the
//     "What's new" button to the matching release notes.
//   • wires copy-to-clipboard affordances on shell `<code>` blocks so
//     users can grab the curl/claude-mcp commands in one click.
//   • speculatively prefetches the primary artifact on link hover so
//     the actual download feels instantaneous.
//
// Everything fails silently — if the fetch is blocked, the detection
// fails, or clipboard access is denied, the page still works from the
// static HTML fallback.

(function () {
  "use strict";

  /* ------------------------------------------------------------------ *
   * 1. Platform-aware primary CTA swap                                  *
   * ------------------------------------------------------------------ */

  const root = document.querySelector("[data-iris-download]");
  if (root) {
    enhanceDownload(root);
  }

  /* ------------------------------------------------------------------ *
   * 2. Copy-to-clipboard for shell code blocks on every page            *
   * ------------------------------------------------------------------ */

  installCopyButtons();

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

      releaseEl.removeAttribute("hidden");
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
  // Copy-to-clipboard for shell code blocks
  // --------------------------------------------------------------------

  function installCopyButtons() {
    if (!navigator.clipboard) return;

    // Material for MkDocs already ships a Copy button on highlighted code
    // blocks — we only add it where it's missing (e.g. inline <code>
    // install commands rendered outside the Highlight extension's scope).
    const sels = [
      "pre > code.language-sh",
      "pre > code.language-bash",
      "pre > code.language-json",
    ];
    const already = ".highlight .md-clipboard";

    document.querySelectorAll(sels.join(",")).forEach((code) => {
      const pre = code.parentElement;
      if (!pre || pre.querySelector(".iris-copy-btn")) return;
      if (pre.closest(already)) return;

      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "iris-copy-btn";
      btn.setAttribute("aria-label", "Copy to clipboard");
      btn.innerHTML = '<span aria-hidden="true">⎘</span><span>Copy</span>';

      btn.addEventListener("click", async (e) => {
        e.preventDefault();
        try {
          await navigator.clipboard.writeText(code.innerText);
          const label = btn.querySelector("span");
          const prior = label.textContent;
          label.textContent = "Copied";
          btn.classList.add("is-ok");
          window.setTimeout(() => {
            label.textContent = prior;
            btn.classList.remove("is-ok");
          }, 1400);
        } catch {
          // Permission denied or non-secure context — swallow.
        }
      });

      pre.style.position = pre.style.position || "relative";
      pre.appendChild(btn);
    });
  }
})();

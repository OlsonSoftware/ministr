// Platform-aware swap for the iris download page.
//
// Reads `data-iris-download` from the landing card, sniffs the visitor's
// OS + arch from `navigator.userAgentData` (when available) or the UA
// string, and rewrites the primary CTA to the matching release artifact.
// Non-macOS visitors still land on a prominent card — just pointed at the
// right file — instead of being sent off to the "Other platforms" fold.

(function () {
  "use strict";

  const root = document.querySelector("[data-iris-download]");
  if (!root) return;

  const version = root.dataset.version || "0.1.0";
  const base = root.dataset.releaseBase || "";
  const primaryLink = root.querySelector("[data-primary-link]");
  const primaryLabel = root.querySelector("[data-primary-label]");
  const sizeEl = root.querySelector("[data-iris-size]");
  const archEl = root.querySelector(".iris-download__arch");
  const subEl = root.querySelector(".iris-download__primary-sub");
  if (!primaryLink || !primaryLabel) return;

  // --- Platform sniff -------------------------------------------------------

  function detect() {
    const ua = navigator.userAgent || "";
    const uaData = navigator.userAgentData;

    let os = "macos";
    let arch = "arm64";

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

    // Arch best-effort. navigator.userAgentData exposes architecture on
    // Chromium; elsewhere we fall back to "arm64" for macOS (overwhelming
    // majority post-2020) and "x64" for Windows/Linux.
    if (uaData && typeof uaData.getHighEntropyValues === "function") {
      return uaData
        .getHighEntropyValues(["architecture", "bitness"])
        .then((hints) => {
          if (hints && hints.architecture === "x86") arch = "x64";
          else if (hints && hints.architecture === "arm") arch = "arm64";
          else if (os === "macos") arch = "arm64";
          else arch = "x64";
          return { os, arch };
        })
        .catch(() => fallbackArch(os, ua));
    }
    return Promise.resolve(fallbackArch(os, ua));
  }

  function fallbackArch(os, ua) {
    let arch = "x64";
    if (os === "macos") {
      arch = /Intel/i.test(ua) ? "x64" : "arm64";
    } else if (/aarch64|arm64/i.test(ua)) {
      arch = "arm64";
    }
    return { os, arch };
  }

  // --- Artifact map ---------------------------------------------------------

  function artifactFor(os, arch) {
    if (os === "macos" && arch === "arm64") {
      return {
        file: `iris-${version}.pkg`,
        label: "Download for macOS — Apple Silicon",
        arch: "Apple Silicon",
        kind: "Signed + notarized .pkg",
        size: "~48 MB",
        minOs: "macOS 13 Ventura or newer",
      };
    }
    if (os === "macos" && arch === "x64") {
      return {
        file: `iris-${version}.pkg`,
        label: "Download for macOS — Intel",
        arch: "Intel Mac",
        kind: "Signed + notarized .pkg (universal)",
        size: "~48 MB",
        minOs: "macOS 13 Ventura or newer",
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
      };
    }
    return null;
  }

  // --- Apply ---------------------------------------------------------------

  detect().then(({ os, arch }) => {
    const art = artifactFor(os, arch);
    if (!art) return;
    primaryLink.href = `${base}/${art.file}`;
    primaryLabel.textContent = art.label;
    if (archEl) archEl.textContent = art.arch;
    if (sizeEl) sizeEl.textContent = art.size;
    if (subEl) {
      subEl.innerHTML = `${art.kind} · <span data-iris-size>${art.size}</span> · ${art.minOs}`;
    }
    root.setAttribute("data-detected-os", os);
    root.setAttribute("data-detected-arch", arch);
  });
})();

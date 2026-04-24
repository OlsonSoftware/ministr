#!/usr/bin/env bash
# Generate PKG installer background PNGs from SVG templates.
# Requires: rsvg-convert (from librsvg, `brew install librsvg`)
#
# macOS Installer backgrounds are displayed at the bottom-left of the window.
# Recommended size: 620x440 (1240x880 @2x).

set -euo pipefail
cd "$(dirname "$0")"

# Light mode background
cat > /tmp/ministr-bg-light.svg <<'SVG'
<svg xmlns="http://www.w3.org/2000/svg" width="620" height="440" viewBox="0 0 620 440">
  <defs>
    <linearGradient id="g" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#f5f5f7" />
      <stop offset="100%" stop-color="#e8e8ed" />
    </linearGradient>
  </defs>
  <rect width="620" height="440" fill="url(#g)" />
  <!-- Subtle geometric accent -->
  <circle cx="540" cy="380" r="120" fill="#d2d2d7" opacity="0.3" />
  <circle cx="570" cy="350" r="60" fill="#86868b" opacity="0.12" />
</svg>
SVG

# Dark mode background
cat > /tmp/ministr-bg-dark.svg <<'SVG'
<svg xmlns="http://www.w3.org/2000/svg" width="620" height="440" viewBox="0 0 620 440">
  <defs>
    <linearGradient id="g" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#1d1d1f" />
      <stop offset="100%" stop-color="#2d2d30" />
    </linearGradient>
  </defs>
  <rect width="620" height="440" fill="url(#g)" />
  <!-- Subtle geometric accent -->
  <circle cx="540" cy="380" r="120" fill="#424245" opacity="0.3" />
  <circle cx="570" cy="350" r="60" fill="#636366" opacity="0.12" />
</svg>
SVG

if command -v rsvg-convert &>/dev/null; then
    rsvg-convert /tmp/ministr-bg-light.svg -o resources/background.png
    rsvg-convert /tmp/ministr-bg-dark.svg -o resources/background-dark.png
    echo "Generated background.png and background-dark.png"
elif command -v sips &>/dev/null; then
    # macOS fallback: use sips (built-in) via intermediate PDF
    # sips can't read SVG directly, so create simple solid PNGs
    # with the native macOS tool as a reasonable fallback.
    echo "rsvg-convert not found. Install with: brew install librsvg"
    echo "Generating placeholder backgrounds with sips..."
    python3 -c "
import struct, zlib

def create_png(path, r, g, b, w=620, h=440):
    def chunk(ctype, data):
        c = ctype + data
        return struct.pack('>I', len(data)) + c + struct.pack('>I', zlib.crc32(c) & 0xffffffff)
    sig = b'\\x89PNG\\r\\n\\x1a\\n'
    ihdr = chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0))
    raw = b''
    for _ in range(h):
        raw += b'\\x00' + bytes([r, g, b]) * w
    idat = chunk(b'IDAT', zlib.compress(raw))
    iend = chunk(b'IEND', b'')
    with open(path, 'wb') as f:
        f.write(sig + ihdr + idat + iend)

create_png('resources/background.png', 245, 245, 247)
create_png('resources/background-dark.png', 29, 29, 31)
"
    echo "Generated placeholder backgrounds (install librsvg for full quality)"
else
    echo "error: no image converter available" >&2
    exit 1
fi

rm -f /tmp/ministr-bg-light.svg /tmp/ministr-bg-dark.svg

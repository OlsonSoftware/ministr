#!/usr/bin/env bash
# Build an SVG sprite from Phosphor icons for the ministr docs site.
#
# Downloads the SVGs we reference from phosphor-icons/core (MIT licensed) and
# concatenates them as <symbol> elements inside a single file referenced by
# <svg><use href="…#name"></use></svg> spans in markdown.
#
# Output: docs/src/assets/icons.svg
#
# Usage:
#   scripts/build-icon-sprite.sh
#
# Re-run whenever you want to add/remove/update icons. Commit the output.

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
output="$repo_root/docs/src/assets/icons.svg"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

# Phosphor raw SVG base URL — main branch, matches latest release usually
base="https://raw.githubusercontent.com/phosphor-icons/core/main/raw"

# Icons we use, in the format: name:weight
# name is the Phosphor icon name (kebab-case), weight is one of:
# thin, light, regular, bold, fill, duotone
# Each entry becomes a <symbol id="name"> (if regular weight) or
# <symbol id="name-fill"> etc. (if weighted variant).
icons=(
  # Core UI (regular weight, id = slug)
  "magnifying-glass:regular"
  "code:regular"
  "graph:regular"
  "cube-focus:regular"
  "cpu:regular"
  "lightning:regular"
  "terminal-window:regular"
  "package:regular"
  "stack:regular"
  "arrow-right:regular"
  "arrow-up-right:regular"
  "check:regular"
  "x:regular"
  "circle:regular"
  "squares-four:regular"
  "circuitry:regular"
  "compass-tool:regular"
  "book-open:regular"
  "gauge:regular"
  "git-branch:regular"

  # Fills for emphasis (id = slug-fill)
  "sparkle:fill"
  "shield-check:fill"
  "check-circle:fill"
)

echo "Downloading ${#icons[@]} Phosphor icons..."

strip_svg() {
  # Extract just the viewBox value and inner content of an <svg> element.
  # Output: "viewBox|inner"
  python3 - <<PY "$1"
import re, sys
with open(sys.argv[1]) as f:
    src = f.read()
m = re.search(r'<svg[^>]*viewBox="([^"]+)"[^>]*>(.*?)</svg>', src, re.DOTALL)
if not m:
    print(f"FAIL: could not parse {sys.argv[1]}", file=sys.stderr)
    sys.exit(1)
viewbox, inner = m.group(1), m.group(2).strip()
print(viewbox + "|" + inner)
PY
}

{
  echo '<?xml version="1.0" encoding="UTF-8"?>'
  echo '<!-- Built from phosphor-icons/core (MIT). Regenerate with scripts/build-icon-sprite.sh -->'
  echo '<svg xmlns="http://www.w3.org/2000/svg" style="display:none">'

  for entry in "${icons[@]}"; do
    IFS=':' read -r name weight <<< "$entry"
    url="$base/$weight/$name${weight:+-$weight}.svg"
    # Regular weight files omit the suffix: "foo.svg" not "foo-regular.svg"
    if [ "$weight" = "regular" ]; then
      url="$base/regular/$name.svg"
      symbol_id="$name"
    else
      url="$base/$weight/$name-$weight.svg"
      symbol_id="$name-$weight"
    fi

    out="$tmpdir/$symbol_id.svg"
    if ! curl -fsSL "$url" -o "$out"; then
      echo "  ✗ $symbol_id (HTTP fail: $url)" >&2
      continue
    fi

    parsed="$(strip_svg "$out")"
    viewbox="${parsed%%|*}"
    inner="${parsed#*|}"

    echo "  <symbol id=\"$symbol_id\" viewBox=\"$viewbox\">"
    printf '    %s\n' "$inner"
    echo "  </symbol>"
    echo "  ✓ $symbol_id" >&2
  done

  echo '</svg>'
} > "$output"

size=$(wc -c < "$output")
count=$(grep -c '<symbol ' "$output" || true)
echo
echo "Wrote $output ($count symbols, $size bytes)"

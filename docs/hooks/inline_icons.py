"""Inline Phosphor icon sprite references at build time.

Problem: Material for MkDocs' navigation.instant URL normalizer runs
    for (const r of document.querySelectorAll('[href], [src]')) { r.href = ... }
which crashes on <use href="..."> elements (SVGUseElement.href is a
read-only SVGAnimatedString). Python-markdown also strips the xlink:
namespace from xlink:href attributes, so switching syntax doesn't help.

Fix: before MkDocs renders markdown to HTML, replace every
    <svg class="icon ..."><use href="assets/icons.svg#NAME"/></svg>
with the actual symbol body from docs/src/assets/icons.svg, adding a
viewBox to the <svg> wrapper. No <use> tags remain in the output,
so Material's attribute walker has nothing to trip on.

Zero runtime cost; all work happens at build time.
"""

from __future__ import annotations

import re
from pathlib import Path

# Compiled once at import time ------------------------------------------------

_SPRITE_PATH = Path(__file__).parent.parent / "src" / "assets" / "icons.svg"

# Maps icon name (without #) to (viewBox_value, inner_svg_markup).
_ICONS: dict[str, tuple[str, str]] = {}


def _load_sprite() -> None:
    """Parse icons.svg into an in-memory {name: (viewBox, inner)} map."""
    if not _SPRITE_PATH.exists():
        return
    sprite = _SPRITE_PATH.read_text(encoding="utf-8")
    pattern = re.compile(
        r'<symbol\s+id="([^"]+)"\s+viewBox="([^"]+)"\s*>(.*?)</symbol>',
        re.DOTALL,
    )
    for match in pattern.finditer(sprite):
        name, view_box, inner = match.group(1), match.group(2), match.group(3).strip()
        _ICONS[name] = (view_box, inner)


_load_sprite()


# Match the full <svg class="icon ..."><use href="...#NAME"/></svg> pattern.
# Not using re.VERBOSE because the pattern contains a literal '#' which
# would be treated as a comment delimiter.
_USE_PATTERN = re.compile(
    r'<svg\s+class="(?P<classes>[^"]*\bicon\b[^"]*)"\s*>'
    r'\s*<use\s+(?:xlink:)?href="[^#]*#(?P<name>[^"]+)"\s*/?\s*>'
    r'(?:\s*</use>)?'
    r'\s*</svg>'
)


def _substitute(match: re.Match[str]) -> str:
    classes = match.group("classes")
    name = match.group("name")
    entry = _ICONS.get(name)
    if not entry:
        # Unknown icon — leave the original markup; build-time warning.
        return match.group(0)
    view_box, inner = entry
    # Inline SVG — fill=currentColor matches our CSS expectation, and the
    # resulting element has no href/src attribute for Material's walker.
    return (
        f'<svg class="{classes}" viewBox="{view_box}" '
        f'fill="currentColor" aria-hidden="true">{inner}</svg>'
    )


def on_page_markdown(markdown: str, page, config, files):  # MkDocs hook signature
    """Inline every icon sprite reference inside markdown before render."""
    if "<use href" not in markdown:
        return markdown
    return _USE_PATTERN.sub(_substitute, markdown)

"""Substitute __IRIS_VERSION__ placeholders with the version from tauri.conf.json.

Keeps the download page's artifact URLs (iris-__IRIS_VERSION__.pkg, etc.) in
sync with whatever's actually shipped in iris-app/src-tauri/tauri.conf.json,
so a `just release 0.2.0` bump doesn't leave the site pointing at a 404'd
iris-0.1.0.pkg on GitHub Releases.

One substitution, no Jinja. Happens at MkDocs page-markdown time, so it
applies before Material's search index is built — placeholders never leak
into search results, rendered text, or hrefs.
"""

from __future__ import annotations

import json
from pathlib import Path

_PLACEHOLDER = "__IRIS_VERSION__"
_TAURI_CONF = (
    Path(__file__).parent.parent.parent
    / "iris-app"
    / "src-tauri"
    / "tauri.conf.json"
)


def _load_version() -> str:
    try:
        with _TAURI_CONF.open(encoding="utf-8") as f:
            data = json.load(f)
        v = data.get("version")
        if isinstance(v, str) and v:
            return v
    except (OSError, json.JSONDecodeError):
        pass
    # Safe fallback keeps the build going even if the canonical source moved.
    return "0.1.0"


_VERSION = _load_version()


def on_page_markdown(markdown: str, page, config, files):  # MkDocs hook signature
    if _PLACEHOLDER not in markdown:
        return markdown
    return markdown.replace(_PLACEHOLDER, _VERSION)


def on_config(config):
    """Expose the version as config.extra.iris_version for templates."""
    extra = dict(config.get("extra") or {})
    extra["iris_version"] = _VERSION
    config["extra"] = extra
    return config

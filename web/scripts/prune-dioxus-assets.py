#!/usr/bin/env python3
"""Remove stale hashed Dioxus assets not reachable from the generated loader."""

from __future__ import annotations

import sys
from pathlib import Path


def main() -> int:
    dist = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("dist")
    assets = dist / "assets"
    index = (dist / "index.html").read_text(encoding="utf-8")
    reachable = {path.name for path in assets.iterdir() if path.name in index}
    for loader in assets.glob("*.js"):
        if loader.name in reachable:
            loader_text = loader.read_text(encoding="utf-8")
            reachable.update(path.name for path in assets.iterdir() if path.name in loader_text)
    for path in assets.iterdir():
        if path.suffix in {".js", ".wasm"} and path.name not in reachable:
            path.unlink()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

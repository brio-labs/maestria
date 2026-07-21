#!/usr/bin/env python3
"""Compute the locked artifact hash required by a visual model profile."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def artifact_hash(profile: dict[str, object], model_dir: Path) -> str:
    artifacts = profile.get("artifacts")
    if not isinstance(artifacts, list) or not artifacts:
        raise ValueError("profile must list at least one artifact")
    digest = hashlib.sha256()
    for item in artifacts:
        if not isinstance(item, str) or not item:
            raise ValueError("profile artifact paths must be non-empty strings")
        path = model_dir / item
        if not path.is_file():
            raise FileNotFoundError(path)
        digest.update(item.encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
    return f"sha256:{digest.hexdigest()}"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--model-dir", required=True, type=Path)
    parser.add_argument(
        "--profiles",
        default=Path(__file__).with_name("visual_model_profiles.json"),
        type=Path,
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    document = json.loads(args.profiles.read_text(encoding="utf-8"))
    profile = document.get("profiles", {}).get(args.profile)
    if not isinstance(profile, dict):
        raise SystemExit(f"unknown visual model profile: {args.profile}")
    print(artifact_hash(profile, args.model_dir))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Read, validate, and update Maestria's canonical workspace version."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SEMVER = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)

INHERITED_PACKAGE_FIELDS = ("version", "edition", "license", "rust-version")


def load_toml(path: Path) -> dict:
    try:
        with path.open("rb") as handle:
            return tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ValueError(f"unable to read {path}: {error}") from error


def workspace_manifests(root: Path) -> list[Path]:
    workspace = load_toml(root / "Cargo.toml").get("workspace", {})
    members = workspace.get("members", [])
    return [root / "Cargo.toml", *(root / member / "Cargo.toml" for member in members)]


def canonical_version(root: Path) -> str:
    manifest = load_toml(root / "Cargo.toml")
    version = manifest.get("workspace", {}).get("package", {}).get("version")
    if not isinstance(version, str) or not SEMVER.fullmatch(version):
        raise ValueError("workspace.package.version must be a valid semantic version")
    return version


def replace_workspace_version(text: str, version: str) -> str:
    section = re.search(
        r"(?ms)^\[workspace\.package\]\n(?P<body>.*?)(?=^\[|\Z)",
        text,
    )
    if section is None:
        raise ValueError("Cargo.toml is missing [workspace.package]")
    body = section.group("body")
    updated, replacements = re.subn(
        r'(?m)^version\s*=\s*"[^"]+"\s*$',
        f'version = "{version}"',
        body,
        count=1,
    )
    if replacements != 1:
        raise ValueError("[workspace.package] must contain exactly one version")
    return text[: section.start("body")] + updated + text[section.end("body") :]



def workspace_package_names(root: Path) -> set[str]:
    names: set[str] = set()
    for manifest_path in workspace_manifests(root):
        package_name = load_toml(manifest_path).get("package", {}).get("name")
        if not isinstance(package_name, str):
            raise ValueError(f"{manifest_path} is missing package.name")
        names.add(package_name)
    return names


def update_lock_versions(text: str, names: set[str], version: str) -> str:
    blocks = list(re.finditer(r"(?ms)^\[\[package\]\]\n.*?(?=^\[\[package\]\]\n|\Z)", text))
    updated_blocks: list[str] = []
    updated_names: set[str] = set()
    cursor = 0
    for block_match in blocks:
        updated_blocks.append(text[cursor : block_match.start()])
        block = block_match.group(0)
        name_match = re.search(r'(?m)^name = "([^"]+)"$', block)
        if name_match is not None and name_match.group(1) in names:
            block, replacements = re.subn(
                r'(?m)^version = "[^"]+"$',
                f'version = "{version}"',
                block,
                count=1,
            )
            if replacements != 1:
                raise ValueError(f"Cargo.lock package {name_match.group(1)} is missing a version")
            updated_names.add(name_match.group(1))
        updated_blocks.append(block)
        cursor = block_match.end()
    updated_blocks.append(text[cursor:])
    missing = names - updated_names
    if missing:
        raise ValueError("Cargo.lock is missing workspace packages: " + ", ".join(sorted(missing)))
    return "".join(updated_blocks)

def check(root: Path, expected: str | None = None) -> list[str]:
    errors: list[str] = []
    try:
        version = canonical_version(root)
    except ValueError as error:
        return [str(error)]

    if expected is not None and version != expected:
        errors.append(f"workspace version is {version}, expected {expected}")

    for manifest_path in workspace_manifests(root):
        try:
            manifest = load_toml(manifest_path)
        except ValueError as error:
            errors.append(str(error))
            continue
        package = manifest.get("package", {})
        relative = manifest_path.relative_to(root)
        for field in INHERITED_PACKAGE_FIELDS:
            if package.get(field) != {"workspace": True}:
                errors.append(f"{relative} must inherit {field}.workspace = true")
    lock_path = root / "Cargo.lock"
    if lock_path.exists():
        try:
            lock = load_toml(lock_path)
        except ValueError as error:
            errors.append(str(error))
        else:
            package_versions = {
                package.get("name"): package.get("version")
                for package in lock.get("package", [])
                if isinstance(package, dict)
            }
            for manifest_path in workspace_manifests(root):
                package_name = load_toml(manifest_path).get("package", {}).get("name")
                if package_versions.get(package_name) != version:
                    relative = manifest_path.relative_to(root)
                    errors.append(f"Cargo.lock does not record {package_name} at {version} ({relative})")

    return errors


def set_version(root: Path, version: str) -> None:
    if SEMVER.fullmatch(version) is None:
        raise ValueError(f"invalid semantic version: {version!r}")
    errors = check(root)
    if errors:
        raise ValueError("version contract is not ready:\n" + "\n".join(f"- {error}" for error in errors))

    manifest_path = root / "Cargo.toml"
    lock_path = root / "Cargo.lock"
    original_manifest = manifest_path.read_text(encoding="utf-8")
    original_lock = lock_path.read_text(encoding="utf-8") if lock_path.exists() else None
    manifest_path.write_text(replace_workspace_version(original_manifest, version), encoding="utf-8")
    if original_lock is not None:
        lock_path.write_text(
            update_lock_versions(original_lock, workspace_package_names(root), version),
            encoding="utf-8",
        )
    try:
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            cwd=root,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        errors = check(root, expected=version)
        if errors:
            raise ValueError("version update did not satisfy the contract:\n" + "\n".join(errors))
    except (OSError, subprocess.CalledProcessError, ValueError):
        manifest_path.write_text(original_manifest, encoding="utf-8")
        if original_lock is not None:
            lock_path.write_text(original_lock, encoding="utf-8")
        raise


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    current = subparsers.add_parser("current", help="print the workspace version")
    current.set_defaults(handler=lambda args: print(canonical_version(ROOT)))

    check_parser = subparsers.add_parser("check", help="validate version inheritance and lockfile metadata")
    check_parser.add_argument("--expected", help="also require this semantic version")
    check_parser.set_defaults(handler=lambda args: run_check(args.expected))

    set_parser = subparsers.add_parser("set", help="set the workspace version and refresh Cargo.lock")
    set_parser.add_argument("version")
    set_parser.set_defaults(handler=lambda args: run_set(args.version))
    return parser


def run_check(expected: str | None) -> int:
    errors = check(ROOT, expected=expected)
    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"version contract is valid ({canonical_version(ROOT)})")
    return 0


def run_set(version: str) -> int:
    try:
        set_version(ROOT, version)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"updated workspace version to {version}")
    return 0


def main() -> int:
    args = build_parser().parse_args()
    return args.handler(args) or 0


if __name__ == "__main__":
    raise SystemExit(main())

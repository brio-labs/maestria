#!/usr/bin/env python3
"""Verify the search-only Debian package metadata and payload."""

from pathlib import Path
import re
import subprocess
import sys
import tarfile

PACKAGE_NAME = "io-github-briolabs-maestria-search"
BINARY_PATH = "usr/bin/maestria-search"
LICENSE_PATH = f"usr/share/doc/{PACKAGE_NAME}/LICENSE"
ALLOWED_FILES = {BINARY_PATH, LICENSE_PATH}
ALLOWED_DIRECTORIES = {
    ".",
    "usr",
    "usr/bin",
    "usr/share",
    "usr/share/doc",
    f"usr/share/doc/{PACKAGE_NAME}",
}
EXPECTED_DEPENDENCIES = {"libc6", "libgcc-s1"}


def fail(message: str) -> None:
    raise SystemExit(f"search package verification failed: {message}")


def run(arguments: list[str]) -> str:
    try:
        result = subprocess.run(arguments, check=True, text=True, capture_output=True)
    except FileNotFoundError as error:
        fail(f"required package tool is unavailable: {error.filename}")
    except subprocess.CalledProcessError as error:
        detail = error.stderr.strip() or error.stdout.strip()
        fail(f"{' '.join(arguments)} failed: {detail}")
    return result.stdout


def control_fields(package: Path) -> dict[str, str]:
    fields: dict[str, str] = {}
    current: str | None = None
    for line in run(["dpkg-deb", "--field", str(package)]).splitlines():
        if line[:1].isspace():
            if current is None:
                fail("Debian control metadata starts with an orphan continuation line")
            fields[current] += "\n" + line[1:]
            continue
        name, separator, value = line.partition(":")
        if not separator or not name or name in fields:
            fail(f"malformed or duplicate Debian control field: {line!r}")
        current = name
        fields[name] = value.strip()
    return fields


def verify_dependencies(fields: dict[str, str]) -> None:
    dependencies = fields.get("Depends", "")
    groups = [group.strip() for group in dependencies.split(",") if group.strip()]
    names: list[str] = []
    for group in groups:
        if "|" in group:
            fail(f"alternative runtime dependency is not permitted: {group}")
        match = re.fullmatch(r"([a-z0-9][a-z0-9+.-]*)(?:\s*\([^()]+\))?", group)
        if match is None:
            fail(f"unrecognized runtime dependency expression: {group}")
        names.append(match.group(1))
    if set(names) != EXPECTED_DEPENDENCIES or len(names) != len(EXPECTED_DEPENDENCIES):
        fail(
            "expected exactly the libc6 and libgcc-s1 runtime dependencies, "
            f"got {dependencies or '(no Depends field)'}"
        )
    for field in ("Pre-Depends", "Recommends", "Suggests"):
        if fields.get(field, "").strip():
            fail(f"unexpected {field} package dependencies: {fields[field]}")


def normalized_path(path: str) -> str:
    while path.startswith("./"):
        path = path[2:]
    return path.rstrip("/") or "."


def verify_payload(package: Path) -> None:
    try:
        process = subprocess.Popen(
            ["dpkg-deb", "--fsys-tarfile", str(package)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except FileNotFoundError as error:
        fail(f"required package tool is unavailable: {error.filename}")
    assert process.stdout is not None
    files: list[str] = []
    violations: list[str] = []
    try:
        with tarfile.open(fileobj=process.stdout, mode="r|*") as archive:
            for member in archive:
                path = normalized_path(member.name)
                if member.isdir():
                    if path not in ALLOWED_DIRECTORIES:
                        violations.append(f"unexpected payload directory {path}")
                    continue
                if not member.isfile():
                    violations.append(f"unexpected non-regular payload entry {path}")
                    continue
                files.append(path)
                if path not in ALLOWED_FILES:
                    violations.append(f"unexpected payload file {path}")
                elif member.size == 0:
                    violations.append(f"empty payload file {path}")
                elif path == BINARY_PATH and member.mode & 0o111 == 0:
                    violations.append(f"search executable is not executable: {path}")
                elif path == LICENSE_PATH and member.mode & 0o111:
                    violations.append(f"license notice is unexpectedly executable: {path}")
    except (tarfile.TarError, OSError) as error:
        if process.poll() is None:
            process.kill()
        process.communicate()
        fail(f"cannot inspect Debian payload: {error}")
    stderr = process.communicate()[1]
    if process.returncode != 0:
        fail(f"dpkg-deb could not read package payload: {stderr.decode(errors='replace').strip()}")
    if files.count(BINARY_PATH) != 1 or files.count(LICENSE_PATH) != 1 or set(files) != ALLOWED_FILES:
        violations.append(f"expected only one executable and its license notice, got {files}")
    if violations:
        fail("; ".join(violations))


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: verify-search-packages.py PACKAGE_DIRECTORY")
    package_dir = Path(sys.argv[1]).resolve()
    if not package_dir.is_dir():
        fail(f"package directory does not exist: {package_dir}")
    packages = sorted(package_dir.glob("*.deb"))
    if len(packages) != 1:
        fail(f"expected exactly one Debian package in {package_dir}, found {len(packages)}")

    package = packages[0]
    fields = control_fields(package)
    if fields.get("Package") != PACKAGE_NAME:
        fail(f"expected Package: {PACKAGE_NAME}, got {fields.get('Package')!r}")
    if fields.get("Architecture") != "amd64":
        fail(f"expected Architecture: amd64, got {fields.get('Architecture')!r}")
    if not fields.get("Version"):
        fail("Debian package has no Version field")
    verify_dependencies(fields)
    verify_payload(package)
    print(
        f"Verified {package.name}: {PACKAGE_NAME} amd64, libc6/libgcc-s1 dependencies, "
        "search executable and license only (no launcher, Studio, desktop, or model assets)"
    )


if __name__ == "__main__":
    main()

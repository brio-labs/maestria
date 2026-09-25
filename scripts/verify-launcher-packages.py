#!/usr/bin/env python3
"""Inspect the generated Debian and AppImage launcher packages."""

from pathlib import Path
import os
import re
import subprocess
import sys
import tempfile

APP_ID = "io.github.briolabs.Maestria.Launcher"
PRODUCT_NAME = "Sillage Launcher"
BINARY_NAME = "maestria-launcher"
PACKAGE_NAME = "io-github-briolabs-maestria-launcher"
DESKTOP_ID = f"{APP_ID}.desktop"
RUNTIME_DEPENDENCIES = {
    "libglib2.0-0t64",
    "libdbus-1-3",
    "libx11-6",
    "libx11-xcb1",
    "libxcb1",
    "libxkbcommon0",
    "libxkbcommon-x11-0",
    "libwayland-client0",
    "libwayland-cursor0",
    "libwayland-egl1",
    "libxcursor1",
    "libxi6",
    "libxrandr2",
    "libxinerama1",
    "libxfixes3",
    "libxext6",
    "libfontconfig1",
    "libfreetype6",
    "wl-clipboard",
}

OPTIONAL_COMPONENT_PACKAGES = {
    "io-github-briolabs-maestria-search",
    "io-github-briolabs-maestria-extension-worker",
}
PACKAGE_RELATION_FIELDS = (
    "Pre-Depends",
    "Depends",
    "Recommends",
    "Suggests",
    "Enhances",
    "Breaks",
    "Conflicts",
    "Provides",
    "Replaces",
)


def fail(message: str) -> None:
    raise SystemExit(message)


def run(arguments: list[str], *, cwd: Path | None = None) -> str:
    result = subprocess.run(
        arguments,
        cwd=cwd,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    return result.stdout


def verify_component_independence(package: Path) -> None:
    fields: dict[str, str] = {}
    current_field: str | None = None
    for line in run(["dpkg-deb", "--field", str(package)]).splitlines():
        if line[:1].isspace():
            if current_field in fields:
                fields[current_field] += "\n" + line[1:]
            continue
        name, separator, value = line.partition(":")
        current_field = name if separator else None
        if current_field in PACKAGE_RELATION_FIELDS:
            fields[current_field] = value.strip()

    violations: list[str] = []
    for field in PACKAGE_RELATION_FIELDS:
        relation = fields.get(field, "")
        declared = {
            match.group(1)
            for match in re.finditer(r"(?:^|[,|]\s*)([a-z0-9][a-z0-9+.-]*)", relation)
        }
        coupled = sorted(OPTIONAL_COMPONENT_PACKAGES & declared)
        if coupled:
            violations.append(f"{field}: {', '.join(coupled)}")
    if violations:
        fail(
            "launcher package must remain independent of separately packaged "
            f"search and extension worker ({'; '.join(violations)})"
        )


def read_desktop(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if line and not line.startswith(("#", "[")) and "=" in line:
            key, value = line.split("=", 1)
            values[key] = value
    return values


def verify_desktop(root: Path, relative_path: str) -> None:
    path = root / relative_path
    if not path.is_file():
        fail(f"missing desktop entry: {path}")
    values = read_desktop(path)
    expected = {
        "Type": "Application",
        "Name": PRODUCT_NAME,
        "Exec": f"{BINARY_NAME} --activate",
        "Icon": BINARY_NAME,
        "Terminal": "false",
        "Categories": "Utility;",
        "StartupWMClass": APP_ID,
    }
    for key, value in expected.items():
        if values.get(key) != value:
            fail(f"{path}: expected {key}={value!r}, got {values.get(key)!r}")


def verify_tree(root: Path, *, appimage: bool) -> None:
    verify_desktop(root, f"usr/share/applications/{DESKTOP_ID}")
    if appimage:
        # cargo-packager's AppRun integration expects an entry named after the
        # executable; the separately named entry above is the desktop ID.
        verify_desktop(root, f"usr/share/applications/{BINARY_NAME}.desktop")

    binary = root / "usr/bin" / BINARY_NAME
    if not binary.is_file() or not os.access(binary, os.X_OK):
        fail(f"missing executable launcher in package: {binary}")
    required_glibc = {
        (int(match.group(1)), int(match.group(2)))
        for match in re.finditer(
            r"\bGLIBC_(\d+)\.(\d+)\b",
            run(["readelf", "--version-info", str(binary)]),
        )
    }
    if required_glibc and max(required_glibc) > (2, 39):
        major, minor = max(required_glibc)
        fail(
            f"{binary} requires GLIBC_{major}.{minor}, newer than the "
            "Ubuntu 24.04 (glibc 2.39) package runtime"
        )

    icon = root / "usr/share/icons/hicolor/128x128/apps" / f"{BINARY_NAME}.png"
    if not icon.is_file():
        fail(f"missing 128x128 launcher icon: {icon}")

    notice = root / "usr/share/doc" / PACKAGE_NAME / "THIRD_PARTY_NOTICES.txt"
    if not notice.is_file() or "Slint 1.18.1" not in notice.read_text(encoding="utf-8"):
        fail(f"missing packaged Slint 1.18.1 notice: {notice}")


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: verify-launcher-packages.py PACKAGE_DIRECTORY")

    package_dir = Path(sys.argv[1]).resolve()
    debs = sorted(package_dir.glob("*.deb"))
    appimages = sorted(package_dir.glob("*.AppImage"))
    if len(debs) != 1 or len(appimages) != 1:
        fail(
            f"expected one Debian package and one AppImage in {package_dir}; "
            f"found {len(debs)} debs and {len(appimages)} AppImages"
        )

    package = debs[0]
    package_name = run(["dpkg-deb", "--field", str(package), "Package"]).strip()
    if package_name != PACKAGE_NAME:
        fail(f"unexpected Debian package identity: {package_name!r}")

    verify_component_independence(package)

    maintainer = run(["dpkg-deb", "--field", str(package), "Maintainer"]).strip()
    if maintainer != "Brio Labs <contact@brio.build>":
        fail(f"unexpected Debian maintainer: {maintainer!r}")
    depends = run(["dpkg-deb", "--field", str(package), "Depends"]).strip()
    declared = {
        match.group(1)
        for match in re.finditer(r"(?:^|,\s*)([a-z0-9][a-z0-9+.-]*)", depends)
    }
    missing = sorted(RUNTIME_DEPENDENCIES - declared)
    if missing:
        fail(f"Debian package omits runtime dependencies: {', '.join(missing)}")

    with tempfile.TemporaryDirectory(prefix="maestria-launcher-package-check-") as temporary:
        root = Path(temporary)
        deb_root = root / "deb"
        deb_root.mkdir()
        run(["dpkg-deb", "--extract", str(package), str(deb_root)])
        verify_tree(deb_root, appimage=False)

        image_dir = root / "appimage"
        image_dir.mkdir()
        appimage = appimages[0].resolve()
        if not os.access(appimage, os.X_OK):
            fail(f"AppImage is not executable: {appimage}")
        run([str(appimage), "--appimage-extract"], cwd=image_dir)
        appimage_root = image_dir / "squashfs-root"
        if not (appimage_root / "AppRun").is_file():
            fail(f"AppImage extraction lacks AppRun: {appimage_root}")
        if not (appimage_root / f"{BINARY_NAME}.desktop").is_file():
            fail(f"AppImage extraction lacks its root desktop entry: {appimage_root}")
        verify_tree(appimage_root, appimage=True)

    print(
        f"Verified {package.name}: Debian identity, runtime dependencies, "
        "no search/worker coupling, desktop ID, icon, notices, and Ubuntu 24.04 "
        "glibc ABI"
    )
    print(
        f"Verified {appimages[0].name}: AppImage payload, desktop ID, icon, "
        "notices, and Ubuntu 24.04 glibc ABI"
    )


if __name__ == "__main__":
    main()

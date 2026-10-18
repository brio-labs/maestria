"""Shared fixture for the philosophy_check test files."""

from __future__ import annotations

import unittest
from pathlib import Path

from philosophy_check import contract_tests, shared


class PhilosophyCheckFixture(unittest.TestCase):

    def setUp(self) -> None:
        self._old_globals = {
            "ROOT": shared.ROOT,
            "THIS_SCRIPT": shared.THIS_SCRIPT,
            "DOMAIN_ROOT": shared.DOMAIN_ROOT,
            "DOMAIN_SRC": shared.DOMAIN_SRC,
            "DOMAIN_MANIFEST": shared.DOMAIN_MANIFEST,
            "KERNEL_ROOTS": shared.KERNEL_ROOTS,
        }
        self._old_responsibility_map_paths = (
            contract_tests.RESPONSIBILITY_MAP_PATHS
        )

    def tearDown(self) -> None:
        for name, value in self._old_globals.items():
            setattr(shared, name, value)
        contract_tests.RESPONSIBILITY_MAP_PATHS = (
            self._old_responsibility_map_paths
        )

    def configure_root(self, root: Path) -> None:
        kernel_root = root / "crates" / "kernel"
        domain_root = kernel_root / "sillage-domain"
        setattr(shared, "ROOT", root)
        setattr(
            shared, "THIS_SCRIPT", root / "scripts" / "philosophy-check.py"
        )
        setattr(shared, "DOMAIN_ROOT", domain_root)
        setattr(shared, "DOMAIN_SRC", domain_root / "src")
        setattr(shared, "DOMAIN_MANIFEST", domain_root / "Cargo.toml")
        setattr(
            shared,
            "KERNEL_ROOTS",
            tuple(
                kernel_root / name
                for name in ("sillage-domain", "sillage-governance", "sillage-ports")
            ),
        )
        contract_tests.RESPONSIBILITY_MAP_PATHS = frozenset()
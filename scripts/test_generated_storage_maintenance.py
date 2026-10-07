from __future__ import annotations

import fcntl
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("generated-storage-maintenance.py")
SPEC = importlib.util.spec_from_file_location("generated_storage_maintenance", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("unable to load generated-storage-maintenance.py")
MAINTENANCE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MAINTENANCE
SPEC.loader.exec_module(MAINTENANCE)


def statvfs_state(available: int, total: int = 100) -> SimpleNamespace:
    return SimpleNamespace(f_blocks=total, f_frsize=1, f_bavail=available)


def arguments(
    target: Path,
    *,
    current: Path | None = None,
    retired: list[Path] | None = None,
    cargo_targets: list[Path] | None = None,
    leases: list[Path] | None = None,
    apply: bool = False,
    high: float = 15.0,
    low: float = 10.0,
    minimum_free: float = 10.0,
) -> SimpleNamespace:
    return SimpleNamespace(
        target_root=str(target),
        current_root=str(current or target / "current"),
        pinned_root=[],
        lease_root=[str(path) for path in leases or []],
        retired_root=[str(path) for path in retired or []],
        cargo_target_root=[str(path) for path in cargo_targets or []],
        high_watermark=high,
        low_watermark=low,
        minimum_free=minimum_free,
        apply=apply,
    )


def changing_capacity(*available: int):
    states = iter(available)
    last = available[-1]

    def measure(_descriptor: int) -> SimpleNamespace:
        nonlocal last
        last = next(states, last)
        return statvfs_state(last)

    return measure


class GeneratedStorageMaintenanceTests(unittest.TestCase):
    def test_high_watermark_boundary_is_strict_and_default_is_read_only(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            target.mkdir()
            current = target / "current"
            current.mkdir()
            retired = target / "retired-generation"
            retired.mkdir()
            payload = retired / "payload.bin"
            payload.write_bytes(b"x" * 15)
            with patch.object(MAINTENANCE.os, "fstatvfs", return_value=statvfs_state(90)):
                result, code = MAINTENANCE.maintain(
                    arguments(target, current=current, retired=[retired])
                )
            self.assertEqual(code, 0)
            self.assertFalse(result["high_watermark_triggered"])
            self.assertEqual(result["target_apparent_bytes_before"], 15)
            self.assertTrue(payload.exists())

    def test_dry_run_reports_whole_target_pressure_without_deleting(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            target.mkdir()
            current = target / "current-runtime"
            current.mkdir()
            (current / "package.deb").write_bytes(b"latest package")
            unknown = target / "unclassified"
            unknown.mkdir()
            (unknown / "large.bin").write_bytes(b"u" * 1024)
            retired = target / "retired-generation"
            retired.mkdir()
            payload = retired / "payload.bin"
            payload.write_bytes(b"r" * 16)
            with patch.object(MAINTENANCE.os, "fstatvfs", return_value=statvfs_state(90)):
                result, code = MAINTENANCE.maintain(
                    arguments(target, current=current, retired=[retired])
                )
            self.assertEqual(code, 2)
            self.assertEqual(result["status"], "dry-run-maintenance-required")
            self.assertTrue(result["high_watermark_triggered"])
            self.assertGreater(result["target_apparent_bytes_before"], 15)
            self.assertEqual(result["unknown_root_count"], 1)
            self.assertEqual(result["pruned_roots"], [])
            self.assertTrue(payload.exists())

    def test_apply_preserves_current_lease_unknown_and_reports_unresolved_watermark(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            target.mkdir()
            current = target / "current-runtime"
            current.mkdir()
            current_file = current / "accepted-package.deb"
            current_file.write_bytes(b"current")
            lease = target / "live-input"
            lease.mkdir()
            lease_file = lease / "producer-plan.json"
            lease_file.write_bytes(b"leased")
            unknown = target / "unclassified"
            unknown.mkdir()
            unknown_file = unknown / "retained.bin"
            unknown_file.write_bytes(b"u" * 11)
            retired = target / "old-producer"
            retired.mkdir()
            (retired / "generated.bin").write_bytes(b"r" * 16)
            with patch.object(
                MAINTENANCE.os,
                "fstatvfs",
                side_effect=changing_capacity(20, 23, 24),
            ):
                result, code = MAINTENANCE.maintain(
                    arguments(
                        target,
                        current=current,
                        retired=[retired, lease],
                        leases=[lease],
                        apply=True,
                    )
                )
            self.assertEqual(code, 2)
            self.assertEqual(result["status"], "maintenance-blocked")
            self.assertTrue(result["capacity_ok"])
            self.assertFalse(retired.exists())
            self.assertEqual(result["pruned_roots"], ["old-producer"])
            self.assertEqual(current_file.read_bytes(), b"current")
            self.assertEqual(lease_file.read_bytes(), b"leased")
            self.assertEqual(unknown_file.read_bytes(), b"u" * 11)
            self.assertEqual(result["unknown_root_count"], 1)
            self.assertFalse(result["watermark_ok"])
            self.assertEqual(result["free_bytes_before"], 20)
            self.assertEqual(result["post_sync_free_bytes"], 24)
            self.assertEqual(result["observed_free_delta_bytes"], 4)

    def test_capacity_gate_uses_changing_post_sync_statvfs_not_candidate_sizes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            target.mkdir()
            current = target / "current"
            current.mkdir()
            retired = target / "retired-generation"
            retired.mkdir()
            (retired / "payload.bin").write_bytes(b"x" * 16)
            with patch.object(
                MAINTENANCE.os,
                "fstatvfs",
                side_effect=changing_capacity(5, 8, 8),
            ):
                result, code = MAINTENANCE.maintain(
                    arguments(target, current=current, retired=[retired], apply=True)
                )
            self.assertEqual(code, 2)
            self.assertEqual(result["status"], "capacity-blocked")
            self.assertFalse(retired.exists())
            self.assertEqual(result["free_bytes_before"], 5)
            self.assertEqual(result["post_sync_free_bytes"], 8)
            self.assertEqual(result["observed_free_delta_bytes"], 3)
            self.assertFalse(result["capacity_ok"])

    def test_unknown_target_pressure_blocks_when_no_owned_root_can_be_deleted(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            target.mkdir()
            current = target / "current-runtime"
            current.mkdir()
            (current / "accepted.deb").write_bytes(b"package")
            unknown = target / "unknown-root"
            unknown.mkdir()
            data = unknown / "preserve.bin"
            data.write_bytes(b"u" * 1024)
            with patch.object(MAINTENANCE.os, "fstatvfs", return_value=statvfs_state(90, 1_000_000)):
                result, code = MAINTENANCE.maintain(
                    arguments(target, current=current, apply=True, high=0.01, low=0.005, minimum_free=0)
                )
            self.assertEqual(code, 2)
            self.assertEqual(result["status"], "maintenance-blocked")
            self.assertEqual(result["unknown_root_count"], 1)
            self.assertEqual(result["unknown_apparent_bytes"], 1024)
            self.assertEqual(result["target_apparent_bytes_after"], result["target_apparent_bytes_before"])
            self.assertEqual(data.read_bytes(), b"u" * 1024)

    def test_symlink_candidate_and_nested_external_link_never_follow_external_data(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            target = base / "target"
            target.mkdir()
            current = target / "current"
            current.mkdir()
            outside = base / "outside"
            outside.mkdir()
            outside_file = outside / "keep.txt"
            outside_file.write_text("preserve", encoding="utf-8")
            (target / "outside-alias").symlink_to(outside, target_is_directory=True)
            with self.assertRaises(MAINTENANCE.UnsafeTree):
                MAINTENANCE.maintain(
                    arguments(target, current=current, retired=[target / "outside-alias"], apply=True)
                )
            self.assertEqual(outside_file.read_text(encoding="utf-8"), "preserve")

            retired = target / "retired-with-link"
            retired.mkdir()
            payload = retired / "ordinary.bin"
            payload.write_bytes(b"x" * 16)
            (retired / "external-link").symlink_to(outside, target_is_directory=True)
            with patch.object(MAINTENANCE.os, "fstatvfs", side_effect=changing_capacity(90, 91, 92)):
                result, code = MAINTENANCE.maintain(
                    arguments(target, current=current, retired=[retired], apply=True)
                )
            self.assertEqual(code, 2)
            self.assertEqual(result["status"], "maintenance-blocked")
            self.assertFalse(payload.exists())
            self.assertTrue((retired / "external-link").is_symlink())
            self.assertEqual(outside_file.read_text(encoding="utf-8"), "preserve")

    def test_source_markers_prevent_retired_root_pruning(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            target.mkdir()
            current = target / "current"
            current.mkdir()
            retired = target / "old-project"
            source = retired / "src"
            source.mkdir(parents=True)
            source_file = source / "main.rs"
            source_file.write_bytes(b"x" * 256)
            with patch.object(MAINTENANCE.os, "fstatvfs", return_value=statvfs_state(90, 1_000)):
                result, code = MAINTENANCE.maintain(
                    arguments(target, current=current, retired=[retired], apply=True, high=5, low=2, minimum_free=0)
                )
            self.assertEqual(code, 2)
            self.assertEqual(result["status"], "maintenance-blocked")
            self.assertEqual(source_file.read_bytes(), b"x" * 256)
            self.assertTrue(any("source" in item["reason"] for item in result["preserved_or_skipped"]))

    def test_held_cargo_lock_inside_retired_root_preserves_lock_and_payload(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            target.mkdir()
            current = target / "current"
            current.mkdir()
            retired = target / "retired-cargo-generation"
            profile = retired / "debug"
            profile.mkdir(parents=True)
            lock = profile / ".cargo-lock"
            lock.write_bytes(b"")
            manifest_lock = profile / "Cargo.lock"
            manifest_lock.write_bytes(b"manifest")
            payload = profile / "old-executable"
            payload.write_bytes(b"x" * 32)
            lock_identity = (lock.stat().st_dev, lock.stat().st_ino)
            lock_fd = os.open(lock, os.O_RDONLY)
            try:
                fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with patch.object(MAINTENANCE.os, "fstatvfs", return_value=statvfs_state(90)):
                    result, code = MAINTENANCE.maintain(
                        arguments(target, current=current, retired=[retired], apply=True)
                    )
            finally:
                os.close(lock_fd)
            self.assertEqual(code, 2)
            self.assertEqual(result["status"], "maintenance-blocked")
            self.assertEqual(result["pruned_roots"], [])
            self.assertTrue(payload.exists())
            self.assertTrue(manifest_lock.exists())
            self.assertEqual((lock.stat().st_dev, lock.stat().st_ino), lock_identity)
            self.assertTrue(any("held" in item["reason"] for item in result["preserved_or_skipped"]))

    def test_replaced_cargo_lock_between_classification_and_pruning_preserves_payload(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            target.mkdir()
            current = target / "current"
            current.mkdir()
            retired = target / "retired-generation"
            retired.mkdir()
            lock = retired / ".cargo-lock"
            lock.write_bytes(b"")
            replaced_lock = retired / ".cargo-lock-original"
            payload = retired / "old-output.bin"
            payload.write_bytes(b"x" * 64)
            original_discover = MAINTENANCE._discover_retired

            def replace_after_classification(*args, **kwargs):
                candidate, reason = original_discover(*args, **kwargs)
                if candidate is not None:
                    lock.rename(replaced_lock)
                    lock.write_bytes(b"replacement")
                return candidate, reason

            with patch.object(MAINTENANCE, "_discover_retired", side_effect=replace_after_classification):
                with patch.object(MAINTENANCE.os, "fstatvfs", return_value=statvfs_state(90)):
                    result, code = MAINTENANCE.maintain(
                        arguments(target, current=current, retired=[retired], apply=True, high=10, low=5)
                    )
            self.assertEqual(code, 2)
            self.assertEqual(result["status"], "lock-identity-blocked")
            self.assertTrue(replaced_lock.exists())
            self.assertTrue(lock.exists())
            self.assertEqual(payload.read_bytes(), b"x" * 64)


    def test_unlocked_retired_cargo_tree_removes_contents_but_preserves_lock_inodes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            target.mkdir()
            current = target / "current"
            current.mkdir()
            retired = target / "retired-cargo-generation"
            profile = retired / "debug"
            profile.mkdir(parents=True)
            lock = profile / ".cargo-lock"
            lock.write_bytes(b"")
            manifest_lock = profile / "Cargo.lock"
            manifest_lock.write_bytes(b"")
            canonical_record = profile / "final-first-outcome.json"
            canonical_record.write_bytes(b"sealed")
            payload = profile / "obsolete.o"
            deps = profile / "deps"
            build_output = profile / "build" / "build-script-hash" / "out"
            deps.mkdir()
            build_output.mkdir(parents=True)
            old_dependency = deps / "obsolete-test-binary"
            old_dependency.write_bytes(b"d" * 16)
            generated_source = build_output / "bindings.rs"
            generated_source.write_bytes(b"g" * 32)
            payload.write_bytes(b"x" * 16)
            lock_id = (lock.stat().st_dev, lock.stat().st_ino)
            manifest_id = (manifest_lock.stat().st_dev, manifest_lock.stat().st_ino)
            record_id = (canonical_record.stat().st_dev, canonical_record.stat().st_ino)
            with patch.object(
                MAINTENANCE.os,
                "fstatvfs",
                side_effect=changing_capacity(20, 22, 23),
            ):
                result, code = MAINTENANCE.maintain(
                    arguments(target, current=current, retired=[retired], apply=True)
                )
            self.assertEqual(code, 0)
            self.assertEqual(result["modified_roots"], ["retired-cargo-generation"])
            self.assertEqual(result["pruned_roots"], [])
            self.assertFalse(payload.exists())
            self.assertFalse(old_dependency.exists())
            self.assertFalse(generated_source.exists())
            self.assertTrue(lock.exists())
            self.assertTrue(manifest_lock.exists())
            self.assertTrue(canonical_record.exists())
            self.assertEqual((canonical_record.stat().st_dev, canonical_record.stat().st_ino), record_id)
            self.assertEqual((lock.stat().st_dev, lock.stat().st_ino), lock_id)
            self.assertEqual((manifest_lock.stat().st_dev, manifest_lock.stat().st_ino), manifest_id)

    def test_cargo_incremental_and_fingerprint_caches_prune_but_current_deps_and_build_remain(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            profile = target / "debug"
            incremental = profile / "incremental" / "stale-unit"
            fingerprint = profile / ".fingerprint"
            deps = profile / "deps"
            build = profile / "build"
            current = profile / "current-runtime"
            incremental.mkdir(parents=True)
            fingerprint.mkdir(parents=True)
            deps.mkdir(parents=True)
            build.mkdir(parents=True)
            current.mkdir(parents=True)
            incremental_file = incremental / "module.o"
            incremental_file.write_bytes(b"i" * 16)
            fingerprint_file = fingerprint / "unit.json"
            fingerprint_file.write_bytes(b"f" * 16)
            dependency_binary = deps / "sillage-search"
            dependency_binary.write_bytes(b"d")
            build_output = build / "native-library.a"
            build_output.write_bytes(b"b")
            current_package = current / "accepted.deb"
            current_package.write_bytes(b"c")
            cargo_lock = profile / ".cargo-lock"
            cargo_lock.write_bytes(b"")
            lock_id = (cargo_lock.stat().st_dev, cargo_lock.stat().st_ino)
            with patch.object(
                MAINTENANCE.os,
                "fstatvfs",
                side_effect=changing_capacity(20, 22, 24, 25),
            ):
                result, code = MAINTENANCE.maintain(
                    arguments(target, current=current, cargo_targets=[profile], apply=True)
                )
            self.assertEqual(code, 0)
            self.assertFalse(incremental.exists())
            self.assertFalse(fingerprint_file.exists())
            self.assertEqual(dependency_binary.read_bytes(), b"d")
            self.assertEqual(build_output.read_bytes(), b"b")
            self.assertEqual(current_package.read_bytes(), b"c")
            self.assertEqual((cargo_lock.stat().st_dev, cargo_lock.stat().st_ino), lock_id)
            self.assertEqual(result["free_bytes_before"], 20)
            self.assertEqual(result["post_sync_free_bytes"], 25)
            self.assertTrue(result["watermark_ok"])

    def test_current_descendant_does_not_pin_sibling_cargo_caches(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            profile = target / "debug"
            stale = profile / "incremental" / "old-unit"
            current = profile / "current-runtime"
            stale.mkdir(parents=True)
            current.mkdir(parents=True)
            stale_file = stale / "cache.o"
            stale_file.write_bytes(b"x" * 20)
            package = current / "latest"
            package.write_bytes(b"latest")
            (profile / ".cargo-lock").write_bytes(b"")
            with patch.object(MAINTENANCE.os, "fstatvfs", side_effect=changing_capacity(90, 91, 92)):
                result, code = MAINTENANCE.maintain(
                    arguments(target, current=current, cargo_targets=[profile], apply=True)
                )
            self.assertEqual(code, 0)
            self.assertFalse(stale.exists())
            self.assertEqual(package.read_bytes(), b"latest")
            self.assertEqual(result["pruned_roots"], ["debug/incremental/old-unit"])
    def test_held_profile_lock_preserves_incremental_and_fingerprint_caches(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            profile = target / "debug"
            incremental = profile / "incremental" / "old-unit"
            fingerprint = profile / ".fingerprint" / "old.json"
            incremental.mkdir(parents=True)
            fingerprint.parent.mkdir(parents=True)
            incremental_file = incremental / "cache.o"
            incremental_file.write_bytes(b"i" * 20)
            fingerprint.write_bytes(b"f" * 20)
            current = profile / "current-runtime"
            current.mkdir()
            (current / "latest").write_bytes(b"current")
            lock = profile / ".cargo-lock"
            lock.write_bytes(b"")
            identity = (lock.stat().st_dev, lock.stat().st_ino)
            descriptor = os.open(lock, os.O_RDONLY)
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with patch.object(MAINTENANCE.os, "fstatvfs", return_value=statvfs_state(90)):
                    result, code = MAINTENANCE.maintain(
                        arguments(target, current=current, cargo_targets=[profile], apply=True)
                    )
            finally:
                os.close(descriptor)
            self.assertEqual(code, 2)
            self.assertEqual(result["status"], "maintenance-blocked")
            self.assertTrue(incremental_file.exists())
            self.assertTrue(fingerprint.exists())
            self.assertEqual((lock.stat().st_dev, lock.stat().st_ino), identity)
            self.assertTrue(any("held" in item["reason"] for item in result["preserved_or_skipped"]))


    def test_cargo_descriptors_are_released_when_cache_inventory_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            profile = target / "debug"
            cache = profile / "incremental" / "unit"
            cache.mkdir(parents=True)
            (cache / "module.o").write_bytes(b"compiled")
            current = target / "current"
            current.mkdir()
            lock = profile / ".cargo-lock"
            lock.write_bytes(b"")
            with patch.object(MAINTENANCE, "_tree_apparent", side_effect=MAINTENANCE.MaintenanceError("injected inventory failure")):
                with self.assertRaises(MAINTENANCE.MaintenanceError):
                    MAINTENANCE.maintain(
                        arguments(target, current=current, cargo_targets=[profile], apply=True)
                    )
            descriptor = os.open(lock, os.O_RDONLY)
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            finally:
                os.close(descriptor)

    def test_same_device_bind_mount_boundary_blocks_inventory_without_host_mounts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            target.mkdir()
            current = target / "current"
            current.mkdir()
            mounted = target / "same-device-mounted"
            mounted.mkdir()
            sentinel = mounted / "keep.bin"
            sentinel.write_bytes(b"keep")
            self.assertEqual(mounted.stat().st_dev, target.stat().st_dev)
            anchor = MAINTENANCE.RootAnchor(target)
            target_mount_id = anchor.mount_id
            anchor.close()
            mounted_inode = mounted.stat().st_ino
            original_mount_id = MAINTENANCE._fd_mount_id

            def simulated_mount_id(descriptor: int) -> int:
                if os.fstat(descriptor).st_ino == mounted_inode:
                    return target_mount_id + 1
                return original_mount_id(descriptor)

            with patch.object(MAINTENANCE, "_fd_mount_id", side_effect=simulated_mount_id):
                with patch.object(MAINTENANCE.os, "fstatvfs", return_value=statvfs_state(90)):
                    result, code = MAINTENANCE.maintain(
                        arguments(target, current=current, apply=True, high=1, low=0.5)
                    )
            self.assertEqual(code, 2)
            self.assertEqual(result["status"], "measurement-blocked")
            self.assertEqual(result["mount_boundary_count"], 1)
            self.assertEqual(sentinel.read_bytes(), b"keep")

    def test_ancestor_replacement_after_classification_cannot_redirect_deletion(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            target = base / "target"
            target.mkdir()
            current = target / "current"
            current.mkdir()
            parent = target / "generation"
            retired = parent / "retired"
            retired.mkdir(parents=True)
            payload = retired / "payload.bin"
            payload.write_bytes(b"x" * 64)
            outside = base / "outside"
            outside.mkdir()
            outside_file = outside / "keep.bin"
            outside_file.write_bytes(b"outside")
            original_discover = MAINTENANCE._discover_retired

            def replace_after_classification(*args, **kwargs):
                candidate, reason = original_discover(*args, **kwargs)
                if candidate is not None:
                    parent.rename(target / "generation-original")
                    parent.symlink_to(outside, target_is_directory=True)
                return candidate, reason

            with patch.object(MAINTENANCE, "_discover_retired", side_effect=replace_after_classification):
                with patch.object(MAINTENANCE.os, "fstatvfs", return_value=statvfs_state(90)):
                    result, code = MAINTENANCE.maintain(
                        arguments(target, current=current, retired=[retired], apply=True, high=10, low=5)
                    )
            self.assertEqual(code, 2)
            self.assertEqual(result["status"], "maintenance-blocked")
            retained_payload = target / "generation-original" / "retired" / "payload.bin"
            self.assertTrue(retained_payload.is_file())
            self.assertEqual(retained_payload.read_bytes(), b"x" * 64)

    def test_actual_cli_dry_run_and_apply_leave_unknown_pressure_blocked(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            target.mkdir()
            current = target / "current-r2"
            current.mkdir()
            current_file = current / "accepted-package.deb"
            current_file.write_bytes(b"latest useful package")
            unknown = target / "unclassified"
            unknown.mkdir()
            unknown_file = unknown / "preserve.bin"
            unknown_file.write_bytes(b"u" * (1024 * 1024))
            retired = target / "explicitly-retired-producer"
            retired.mkdir()
            (retired / "generated.bin").write_bytes(b"r" * 65536)
            total = os.statvfs(target).f_blocks * os.statvfs(target).f_frsize
            high = 165.0 / total * 100.0
            low = 82.0 / total * 100.0
            command = [
                sys.executable,
                str(SCRIPT),
                "guard",
                "--target-root",
                str(target),
                "--current-root",
                str(current),
                "--retired-root",
                str(retired),
                "--high-watermark",
                str(high),
                "--low-watermark",
                str(low),
                "--minimum-free",
                "0",
            ]
            dry = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(dry.returncode, 2, dry.stderr + dry.stdout)
            dry_result = json.loads(dry.stdout)
            self.assertEqual(dry_result["status"], "dry-run-maintenance-required")
            self.assertEqual(dry_result["pruned_roots"], [])
            self.assertTrue((retired / "generated.bin").exists())

            applied = subprocess.run(command + ["--apply"], capture_output=True, text=True, check=False)
            self.assertEqual(applied.returncode, 2, applied.stderr + applied.stdout)
            applied_result = json.loads(applied.stdout)
            self.assertEqual(applied_result["status"], "maintenance-blocked")
            self.assertEqual(applied_result["pruned_roots"], ["explicitly-retired-producer"])
            self.assertEqual(applied_result["unknown_root_count"], 1)
            self.assertTrue(applied_result["high_watermark_triggered"])
            self.assertFalse(applied_result["watermark_ok"])
            self.assertFalse(retired.exists())
            self.assertEqual(current_file.read_bytes(), b"latest useful package")
            self.assertEqual(unknown_file.read_bytes(), b"u" * (1024 * 1024))

    def test_actual_cli_dry_run_then_authorized_apply_reaches_low_watermark(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "target"
            target.mkdir()
            current = target / "current-r2"
            current.mkdir()
            current_file = current / "accepted-package.deb"
            current_file.write_bytes(b"latest useful package")
            retired = target / "explicitly-retired-producer"
            retired.mkdir()
            (retired / "generated.bin").write_bytes(b"r" * 8192)
            total = os.statvfs(target).f_blocks * os.statvfs(target).f_frsize
            high = 4096.0 / total * 100.0
            low = 2048.0 / total * 100.0
            command = [
                sys.executable,
                str(SCRIPT),
                "guard",
                "--target-root",
                str(target),
                "--current-root",
                str(current),
                "--retired-root",
                str(retired),
                "--high-watermark",
                str(high),
                "--low-watermark",
                str(low),
                "--minimum-free",
                "0",
            ]
            dry = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(dry.returncode, 2, dry.stderr + dry.stdout)
            self.assertTrue((retired / "generated.bin").exists())
            applied = subprocess.run(command + ["--apply"], capture_output=True, text=True, check=False)
            self.assertEqual(applied.returncode, 0, applied.stderr + applied.stdout)
            result = json.loads(applied.stdout)
            self.assertEqual(result["status"], "ok")
            self.assertTrue(result["watermark_ok"])
            self.assertFalse(retired.exists())
            self.assertEqual(current_file.read_bytes(), b"latest useful package")


if __name__ == "__main__":
    unittest.main()

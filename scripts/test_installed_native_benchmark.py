from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("smoke-installed-native-benchmark.py")
SPEC = importlib.util.spec_from_file_location("installed_native_benchmark", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("unable to load installed native benchmark driver")
BENCHMARK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCHMARK)


class StorageProvenanceTests(unittest.TestCase):
    def provenance(
        self, sources: list[str], member_devices: tuple[str, ...] = ("253:0",)
    ) -> dict:
        mount = {
            "filesystems": [
                {
                    "target": "/",
                    "source": sources[0],
                    "sources": sources,
                    "fstype": "btrfs",
                    "maj:min": "0:29",
                }
            ]
        }
        devices = {
            "blockdevices": [
                {
                    "name": "nvme0n1", "path": "/dev/nvme0n1", "type": "disk",
                    "maj:min": "259:0", "rota": False, "tran": "nvme",
                    "children": [
                        {
                            "name": "nvme0n1p2", "path": "/dev/nvme0n1p2",
                            "type": "part", "maj:min": "259:2", "pkname": "nvme0n1",
                            "children": [
                                {
                                    "name": "ArchinstallVg-root",
                                    "path": "/dev/mapper/ArchinstallVg-root",
                                    "type": "lvm", "maj:min": "253:0",
                                    "pkname": "nvme0n1p2",
                                }
                            ],
                        }
                    ],
                }
            ]
        }

        def capture(arguments: list[str]) -> str:
            return json.dumps(mount if arguments[0] == "findmnt" else devices)

        with tempfile.TemporaryDirectory() as directory:
            devices_dir = Path(directory) / "test-btrfs-uuid" / "devices"
            for index, majmin in enumerate(member_devices):
                member = devices_dir / f"device-{index}"
                member.mkdir(parents=True)
                (member / "dev").write_text(majmin + "\n", encoding="ascii")
            with (
                patch.object(BENCHMARK, "BTRFS_SYSFS_ROOT", Path(directory)),
                patch.object(BENCHMARK, "run_capture", side_effect=capture),
            ):
                return BENCHMARK.storage_provenance(Path("/tmp/benchmark"))

    def test_single_btrfs_source_maps_virtual_mount_to_nvme_disk(self) -> None:
        result = self.provenance(["/dev/mapper/ArchinstallVg-root[/@home/benchmark]"])
        self.assertTrue(result["ssd_established"])
        self.assertEqual(result["btrfs_single_device_source"], "/dev/mapper/ArchinstallVg-root")
        self.assertEqual(result["btrfs_kernel_device_majmin"], "253:0")
        self.assertEqual(result["backing_disk"]["name"], "nvme0n1")

    def test_multi_device_btrfs_does_not_certify_one_ssd_as_all_storage(self) -> None:
        result = self.provenance([
            "/dev/mapper/ArchinstallVg-root[/@home/benchmark]", "/dev/sdb1"
        ])
        self.assertFalse(result["resolved"])
        self.assertNotIn("ssd_established", result)

    def test_kernel_multi_device_btrfs_rejects_incomplete_findmnt_sources(self) -> None:
        result = self.provenance(
            ["/dev/mapper/ArchinstallVg-root[/@home/benchmark]"],
            member_devices=("253:0", "8:16"),
        )
        self.assertFalse(result["resolved"])
        self.assertNotIn("ssd_established", result)

    def test_unavailable_kernel_btrfs_device_list_fails_closed(self) -> None:
        result = self.provenance(
            ["/dev/mapper/ArchinstallVg-root[/@home/benchmark]"],
            member_devices=(),
        )
        self.assertFalse(result["resolved"])
        self.assertNotIn("ssd_established", result)

    def test_unmapped_btrfs_source_does_not_claim_ssd(self) -> None:
        result = self.provenance(["/dev/mapper/other[/@home/benchmark]"])
        self.assertFalse(result["resolved"])
        self.assertNotIn("ssd_established", result)


if __name__ == "__main__":
    unittest.main()

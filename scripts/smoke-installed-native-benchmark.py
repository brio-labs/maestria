#!/usr/bin/python3
"""Prepare and exercise the installed native launcher through private X11/AT-SPI."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import signal
import subprocess
import sys
import threading
import tempfile
import time
from collections import defaultdict
from pathlib import Path
from typing import Any

TEXT_FILE_COUNT = 10_000
TEXT_FILE_CAP_BYTES = 100 * 1024 * 1024
DESKTOP_ENTRY_COUNT = 500
SAMPLES_PER_CLASS = 200
INTERNAL_DEADLINE_MS = 100.0
DEFAULT_SAMPLE_TIMEOUT_MS = 5_000
QUERY_LABEL = "Search applications, commands, files, or calculate"
WINDOW_TITLE = "Sillage Launcher"
COPY_CITATION_LABEL = "Reopen this evidence and copy its citation"
COPY_EXCERPT_LABEL = "Reopen this evidence and copy the returned passage"
REOPEN_DENIALS = (
    "The source changed or access was revoked.",
    "The document could not be reopened; search service may be unavailable.",
)


def fail(message: str) -> "NoReturn":
    raise SystemExit(f"installed native benchmark driver failed: {message}")


def json_write(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def jsonl_append(path: Path, value: Any) -> None:
    with path.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(value, ensure_ascii=False, sort_keys=True) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


def run_capture(arguments: list[str], *, timeout: float = 20.0) -> str:
    result = subprocess.run(
        arguments,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"command returned {result.returncode}: {arguments!r}: "
            f"{result.stderr.strip() or result.stdout.strip()}"
        )
    return result.stdout


def fixture_token(kind: str, index: int, state: str = "") -> str:
    prefix = {
        "cold": "mbenchcold",
        "active": "mbenchactive",
        "edit": "mbenchedit",
        "delete": "mbenchdelete",
    }[kind]
    suffix = f"{state}" if state else ""
    return f"{prefix}{suffix}{index:03d}q"


def fixture_text(token: str, *, revised: bool = False) -> str:
    version = "revised" if revised else "original"
    return (
        f"{version.title()} authorized benchmark passage {token} states that "
        "a cobalt kestrel carries a silver compass beneath northern rain."
    )


def prepare(arguments: argparse.Namespace) -> None:
    approved_root = Path(arguments.approved_root).resolve(strict=True)
    data_home = Path(arguments.data_home).resolve(strict=True)
    output_dir = Path(arguments.output_dir).resolve(strict=True)
    if any(approved_root.iterdir()):
        fail(f"approved root is not empty: {approved_root}")
    application_dir = data_home / "applications"
    if not application_dir.is_dir():
        fail(f"private desktop-entry directory is missing: {application_dir}")
    if any(application_dir.iterdir()):
        fail(f"private desktop-entry directory is not empty: {application_dir}")

    samples: dict[str, list[dict[str, Any]]] = {
        "cold": [],
        "active": [],
        "edit": [],
        "delete": [],
    }
    file_index = 0
    for kind in ("cold", "active", "edit", "delete"):
        for index in range(SAMPLES_PER_CLASS):
            relative_path = f"document-{file_index:05d}.md"
            path = approved_root / relative_path
            if kind == "edit":
                old_token = fixture_token(kind, index, "old")
                new_token = fixture_token(kind, index, "fresh")
                initial_content = fixture_text(old_token)
                final_content = fixture_text(new_token, revised=True)
                sample = {
                    "sample_index": index,
                    "path": str(path),
                    "old_query": old_token,
                    "query": new_token,
                    "initial_content": initial_content,
                    "expected_content": final_content,
                    "expected_citation": f"{path}:1",
                }
            else:
                token = fixture_token(kind, index)
                content = fixture_text(token)
                sample = {
                    "sample_index": index,
                    "path": str(path),
                    "query": token,
                    "initial_content": content,
                    "expected_content": content,
                    "expected_citation": f"{path}:1",
                }
            encoded = (sample["initial_content"] + "\n").encode("utf-8")
            path.write_bytes(encoded)
            sample["initial_sha256"] = hashlib.sha256(encoded).hexdigest()
            sample["initial_size_bytes"] = len(encoded)
            samples[kind].append(sample)
            file_index += 1

    filler = (
        "Approved deterministic corpus filler. This plain text paragraph exists "
        "to exercise native document indexing and retrieval across many small "
        "independent files without adding a matching benchmark query. "
    )
    for index in range(file_index, TEXT_FILE_COUNT):
        path = approved_root / f"document-{index:05d}.md"
        content = (
            f"Filler record {index:05d}.\n" + filler * 4 + f"Ordinal {index:05d}.\n"
        )
        path.write_text(content, encoding="utf-8", newline="\n")
        file_index += 1

    for index in range(DESKTOP_ENTRY_COUNT):
        desktop_path = application_dir / f"native-benchmark-{index:03d}.desktop"
        desktop_path.write_text(
            "[Desktop Entry]\n"
            "Type=Application\n"
            f"Name=Native Benchmark Application {index:03d}\n"
            "Comment=Private deterministic native launcher catalog fixture\n"
            "Exec=/usr/bin/true\n"
            "Terminal=false\n"
            "Categories=Utility;\n",
            encoding="utf-8",
            newline="\n",
        )

    manifest_rows: list[dict[str, Any]] = []
    total_bytes = 0
    for path in sorted(approved_root.glob("*.md")):
        data = path.read_bytes()
        total_bytes += len(data)
        manifest_rows.append(
            {
                "relative_path": path.relative_to(approved_root).as_posix(),
                "size_bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )
    actual_count = sum(1 for path in approved_root.iterdir() if path.is_file())
    if actual_count != TEXT_FILE_COUNT or len(manifest_rows) != TEXT_FILE_COUNT:
        fail(f"fixture has {actual_count} files, expected exactly {TEXT_FILE_COUNT}")
    if total_bytes > TEXT_FILE_CAP_BYTES:
        fail(f"approved text fixture is {total_bytes} bytes, above {TEXT_FILE_CAP_BYTES}")
    desktop_count = sum(1 for path in application_dir.iterdir() if path.is_file())
    if desktop_count != DESKTOP_ENTRY_COUNT:
        fail(f"private catalog has {desktop_count} desktop entries, expected {DESKTOP_ENTRY_COUNT}")

    payload = {
        "schema_version": 1,
        "fixture_definition": "deterministic installed native launcher/search corpus",
        "approved_root": str(approved_root),
        "approved_text_file_count": actual_count,
        "approved_text_file_cap_bytes": TEXT_FILE_CAP_BYTES,
        "approved_text_bytes": total_bytes,
        "approved_text_manifest_sha256": hashlib.sha256(
            json.dumps(manifest_rows, sort_keys=True, separators=(",", ":")).encode("utf-8")
        ).hexdigest(),
        "desktop_entry_directory": str(application_dir),
        "desktop_entry_count": desktop_count,
        "samples_per_class": SAMPLES_PER_CLASS,
        "classes": samples,
        "cold_semantics": "first query in a newly started launcher process; daemon and OS page cache are not claimed cold",
    }
    json_write(output_dir / "fixture.json", payload)
    json_write(output_dir / "fixture-manifest.json", manifest_rows)


def flatten_lsblk(items: list[dict[str, Any]]) -> list[dict[str, Any]]:
    flattened: list[dict[str, Any]] = []
    pending = list(items)
    while pending:
        item = pending.pop()
        flattened.append(item)
        pending.extend(item.get("children") or [])
    return flattened


def storage_provenance(path: Path) -> dict[str, Any]:
    record: dict[str, Any] = {"path": str(path), "resolved": False}
    try:
        raw_mount = run_capture(
            [
                "findmnt",
                "--json",
                "--target",
                str(path),
                "--output",
                "TARGET,SOURCE,FSTYPE,OPTIONS,MAJ:MIN",
            ]
        )
        record["findmnt"] = json.loads(raw_mount)
    except Exception as error:
        record["findmnt_error"] = str(error)
        raw_mount = ""
    try:
        raw_devices = run_capture(
            [
                "lsblk",
                "--json",
                "--bytes",
                "--output",
                "NAME,PATH,TYPE,SIZE,MODEL,ROTA,TRAN,PKNAME,MAJ:MIN",
            ]
        )
        device_data = json.loads(raw_devices)
        record["lsblk"] = device_data
        all_devices = flatten_lsblk(device_data.get("blockdevices", []))
    except Exception as error:
        record["lsblk_error"] = str(error)
        return record
    mounts = record.get("findmnt", {}).get("filesystems", [])
    if len(mounts) != 1:
        record["resolution_error"] = "findmnt did not identify exactly one backing mount"
        return record
    mount = mounts[0]
    record["mount"] = mount
    mount_majmin = mount.get("maj:min") or mount.get("maj:MIN")
    current = next(
        (
            device
            for device in all_devices
            if (device.get("maj:min") or device.get("maj:MIN")) == mount_majmin
        ),
        None,
    )
    if current is None:
        record["resolution_error"] = (
            f"mount source {mount.get('source')} ({mount_majmin}) is not mapped by lsblk"
        )
        return record
    by_name = {str(device.get("name")): device for device in all_devices}
    chain: list[dict[str, Any]] = []
    seen: set[str] = set()
    disk = None
    while current is not None:
        name = str(current.get("name"))
        if name in seen:
            record["resolution_error"] = "lsblk returned a cyclic backing-device chain"
            break
        seen.add(name)
        chain.append(current)
        if current.get("type") == "disk":
            disk = current
            break
        parent = current.get("pkname")
        if not parent:
            break
        parent_name = str(parent).rsplit("/", 1)[-1]
        current = by_name.get(parent_name)
    record["block_device_chain"] = chain
    record["backing_disk"] = disk
    record["resolved"] = disk is not None
    if disk is None:
        record.setdefault(
            "resolution_error",
            "no physical block disk was found in the mount's lsblk parent chain",
        )
        record["ssd_established"] = False
        return record
    rota = disk.get("rota")
    transport = str(disk.get("tran") or "").strip().lower()
    record["ssd_established"] = rota in (0, False, "0") or transport == "nvme"
    record["ssd_evidence"] = {
        "rota": rota,
        "transport": transport or None,
        "rule": "SSD is established only by lsblk ROTA=0 or TRAN=nvme",
    }
    return record


def package_provenance(package_name: str, deb: Path, binary: Path) -> dict[str, Any]:
    fields = {
        key.lower(): run_capture(["dpkg-deb", "--field", str(deb), key]).strip()
        for key in ("Package", "Version", "Architecture", "Maintainer", "Depends")
    }
    if fields.get("package") != package_name:
        fail(f"artifact {deb} reports unexpected package {fields.get('package')!r}")
    installed = run_capture(
        [
            "dpkg-query",
            "--show",
            "--showformat=${Status}|${Version}|${Architecture}",
            package_name,
        ]
    ).strip()
    owner = run_capture(["dpkg-query", "--search", str(binary)]).strip()
    version_check = subprocess.run(
        [str(binary), "--version"],
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=10.0,
    )
    binary_sha256 = hashlib.sha256(binary.read_bytes()).hexdigest()
    with tempfile.TemporaryDirectory(
        prefix="installed-native-artifact-", dir=os.environ.get("TMPDIR")
    ) as temporary_directory:
        run_capture(["dpkg-deb", "--extract", str(deb), temporary_directory], timeout=120.0)
        artifact_binary = Path(temporary_directory) / str(binary).lstrip("/")
        if not artifact_binary.is_file() or artifact_binary.is_symlink():
            fail(f"artifact {deb} does not contain the expected regular binary {binary}")
        artifact_binary_sha256 = hashlib.sha256(artifact_binary.read_bytes()).hexdigest()
    if artifact_binary_sha256 != binary_sha256:
        fail(
            f"installed executable {binary} does not match the exact artifact member in {deb}: "
            f"installed={binary_sha256}, artifact={artifact_binary_sha256}"
        )
    return {
        "package": package_name,
        "artifact_path": str(deb.resolve(strict=True)),
        "artifact_sha256": hashlib.sha256(deb.read_bytes()).hexdigest(),
        "artifact_fields": fields,
        "installed_status_version_architecture": installed,
        "installed_binary": str(binary),
        "binary_owner": owner,
        "artifact_binary_sha256": artifact_binary_sha256,
        "binary_sha256": binary_sha256,
        "installed_binary_matches_artifact": True,
        "binary_version_command": {
            "returncode": version_check.returncode,
            "stdout": version_check.stdout.strip(),
            "stderr": version_check.stderr.strip(),
        },
    }




def provenance(arguments: argparse.Namespace) -> None:
    output_dir = Path(arguments.output_dir).resolve(strict=True)
    approved_root = Path(arguments.approved_root).resolve(strict=True)
    try:
        lscpu = json.loads(run_capture(["lscpu", "--json"]))
    except (json.JSONDecodeError, RuntimeError) as error:
        fail(f"lscpu hardware provenance is unavailable: {error}")
    try:
        free = run_capture(["free", "--bytes"])
    except RuntimeError as error:
        fail(f"physical memory provenance is unavailable: {error}")
    meminfo = Path("/proc/meminfo").read_text(encoding="utf-8")
    cpu_model = next(
        (
            row.get("data")
            for row in lscpu.get("lscpu", [])
            if str(row.get("field", "")).strip().lower() in {"model name:", "model name"}
        ),
        None,
    )
    if not cpu_model:
        fail("lscpu did not report a CPU model name")
    mem_total_kib = next(
        (
            int(line.split()[1])
            for line in meminfo.splitlines()
            if line.startswith("MemTotal:") and len(line.split()) >= 2
        ),
        None,
    )
    if not mem_total_kib:
        fail("/proc/meminfo did not report physical memory")
    mem_total_bytes = mem_total_kib * 1024
    mem_total_gib = mem_total_bytes / (1024**3)
    cgroup: dict[str, str] = {}
    for key, path in (
        ("memory_max_bytes", Path("/sys/fs/cgroup/memory.max")),
        ("memory_current_bytes", Path("/sys/fs/cgroup/memory.current")),
        ("cpu_max", Path("/sys/fs/cgroup/cpu.max")),
    ):
        if path.is_file():
            cgroup[key] = path.read_text(encoding="utf-8").strip()
    memory_limit = cgroup.get("memory_max_bytes", "max")
    effective_mem_bytes = min(
        mem_total_bytes,
        int(memory_limit) if memory_limit.isdecimal() else mem_total_bytes,
    )
    storage = {
        "approved_fixture": storage_provenance(approved_root),
        "benchmark_output": storage_provenance(output_dir),
    }
    ssd_established = all(
        bool(device.get("ssd_established"))
        for device in storage.values()
    )
    reference_failures: list[str] = []
    if effective_mem_bytes < 16 * 1024**3:
        reference_failures.append("effective physical/cgroup memory limit is below 16 GiB")
    if not ssd_established:
        reference_failures.append("SSD storage could not be established from lsblk ROTA/TRAN data")
    hardware = {
        "os_release": platform.freedesktop_os_release(),
        "uname": platform.uname()._asdict(),
        "cpu": {
            "lscpu": lscpu,
            "model_name": str(cpu_model).strip(),
        },
        "memory": {
            "mem_total_bytes_from_proc": mem_total_bytes,
            "mem_total_gib_from_proc": mem_total_gib,
            "effective_mem_bytes": effective_mem_bytes,
            "effective_mem_gib": effective_mem_bytes / (1024**3),
            "free_bytes_output": free,
            "meminfo_selected": "\n".join(
                line
                for line in meminfo.splitlines()
                if line.startswith(("MemTotal:", "MemAvailable:", "SwapTotal:", "SwapFree:"))
            ),
            "cgroup_limits_and_current": cgroup,
        },
        "block_storage": storage,
        "reference_hardware_gate": {
            "required_mem_total_gib": 16.0,
            "observed_effective_mem_gib": effective_mem_bytes / (1024**3),
            "ssd_established_for_fixture_and_output": ssd_established,
            "passed": not reference_failures,
            "failures": reference_failures,
        },
    }
    json_write(output_dir / "hardware.json", hardware)
    launcher = package_provenance(
        arguments.launcher_package,
        Path(arguments.launcher_deb),
        Path(arguments.launcher_binary),
    )
    search = package_provenance(
        arguments.search_package,
        Path(arguments.search_deb),
        Path(arguments.search_binary),
    )
    json_write(
        output_dir / "provenance.json",
        {
            "schema_version": 1,
            "os_release": hardware["os_release"],
            "host_architecture": run_capture(["dpkg", "--print-architecture"]).strip(),
            "launcher": launcher,
            "search": search,
            "reference_hardware_gate": hardware["reference_hardware_gate"],
            "package_installation_mode": "preinstalled exact Debian artifact; this script does not install, upgrade, or remove host packages",
            "runtime_mode": "private XDG/D-Bus/Xvfb; installed search daemon launched only with the private instance directory and approved root",
        },
    )


def search_response(binary: str, subcommand: str, socket_path: str, realm: str, credential: str) -> dict[str, Any]:
    output = run_capture(
        [
            binary,
            subcommand,
            "--socket-path",
            socket_path,
            "--consumer-realm",
            realm,
            "--credential-file",
            credential,
        ],
        timeout=20.0,
    )
    try:
        response = json.loads(output)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"{subcommand} returned non-JSON output: {error}") from error
    if not isinstance(response, dict):
        raise RuntimeError(f"{subcommand} response is not a JSON object")
    return response


def process_identity(pid: int) -> tuple[int, str]:
    process_dir = Path("/proc") / str(pid)
    stat = (process_dir / "stat").read_text(encoding="ascii")
    remainder = stat[stat.rfind(")") + 2 :].split()
    return int(remainder[19]), os.readlink(process_dir / "exe")


def register_resource_process(targets_path: Path, pid: int, expected_binary: str) -> None:
    deadline = time.monotonic() + 5.0
    executable = ""
    while time.monotonic() < deadline:
        try:
            start_ticks, executable = process_identity(pid)
        except FileNotFoundError as error:
            raise RuntimeError(f"resource target PID {pid} exited before executable registration") from error
        if executable.removesuffix(" (deleted)") == expected_binary:
            target = {
                "pid": pid,
                "start_ticks": start_ticks,
                "executable": expected_binary,
            }
            with targets_path.open("a", encoding="utf-8") as stream:
                stream.write(json.dumps(target, sort_keys=True) + "\n")
                stream.flush()
            return
        time.sleep(0.01)
    raise RuntimeError(
        f"resource target PID {pid} executable {executable!r} did not match installed binary {expected_binary!r}"
    )


def record_process(arguments: argparse.Namespace) -> None:
    register_resource_process(
        Path(arguments.resource_targets_file),
        arguments.pid,
        arguments.expected_binary,
    )

def wait_index(arguments: argparse.Namespace) -> None:
    output_dir = Path(arguments.output_dir).resolve(strict=True)
    started = time.monotonic()
    deadline = started + arguments.timeout_seconds
    poll_count = 0
    errors: list[str] = []
    final_status: dict[str, Any] | None = None
    final_consumer_status: dict[str, Any] | None = None
    error: str | None = None
    try:
        final_consumer_status = search_response(
            arguments.search_binary,
            "status",
            arguments.socket_path,
            arguments.consumer_realm,
            arguments.credential_file,
        )
        if final_consumer_status.get("type") != "status" or not isinstance(final_consumer_status.get("data"), dict):
            raise RuntimeError("private consumer status did not return typed status JSON")
    except Exception as failure:  # Persist the failed authorization preflight as provenance.
        error = str(failure)
    if error is None:
        while time.monotonic() < deadline:
            poll_count += 1
            try:
                status = search_response(
                    arguments.search_binary,
                    "indexing-status",
                    arguments.socket_path,
                    arguments.consumer_realm,
                    arguments.credential_file,
                )
                final_status = status
                data = status.get("data")
                if status.get("type") != "indexing_status" or not isinstance(data, dict):
                    errors.append("indexing-status returned an unexpected response shape")
                elif (
                    data.get("approved_root_count") == 1
                    and data.get("indexed_file_count") == arguments.expected_files
                    and data.get("pending_file_count") == 0
                    and data.get("scanning") is False
                    and data.get("last_scan_error") is False
                ):
                    error = None
                    break
            except Exception as failure:
                errors.append(str(failure))
            time.sleep(0.5)
        else:
            error = f"index did not settle with exactly {arguments.expected_files} approved Markdown files"
    elapsed = (time.monotonic() - started) * 1000.0
    report = {
        "schema_version": 1,
        "expected_approved_root_count": 1,
        "expected_indexed_file_count": arguments.expected_files,
        "poll_interval_ms": 500,
        "poll_count": poll_count,
        "elapsed_ms": elapsed,
        "consumer_status": final_consumer_status,
        "final_indexing_status": final_status,
        "recent_poll_errors": errors[-20:],
        "passed": error is None,
        "error": error,
    }
    json_write(output_dir / "indexing-wait.json", report)
    if error is not None:
        fail(f"installed search indexing/authorization preflight did not pass: {error}")


def proc_snapshot(targets_path: Path) -> dict[str, Any]:
    clock_ticks = os.sysconf(os.sysconf_names["SC_CLK_TCK"])
    page_size = os.sysconf("SC_PAGE_SIZE")
    stat_fields = Path("/proc/stat").read_text(encoding="ascii").splitlines()[0].split()
    cpu_ticks = sum(int(value) for value in stat_fields[1:])
    mem_values: dict[str, int] = {}
    for line in Path("/proc/meminfo").read_text(encoding="ascii").splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[0].rstrip(":") in {"MemTotal", "MemAvailable"}:
            mem_values[parts[0].rstrip(":")] = int(parts[1]) * 1024
    target_by_pid: dict[int, dict[str, Any]] = {}
    with targets_path.open(encoding="utf-8") as stream:
        for line in stream:
            if not line.endswith("\n"):
                break
            target = json.loads(line)
            target_by_pid[int(target["pid"])] = target
    processes: list[dict[str, Any]] = []
    for pid, target in target_by_pid.items():
        entry = Path("/proc") / str(pid)
        try:
            stat = (entry / "stat").read_text(encoding="ascii")
            remainder = stat[stat.rfind(")") + 2 :].split()
            start_ticks = int(remainder[19])
            if start_ticks != int(target["start_ticks"]):
                continue
            executable = os.readlink(entry / "exe")
            if executable.removesuffix(" (deleted)") != target["executable"]:
                raise RuntimeError(
                    f"resource target PID {pid} changed executable from {target['executable']!r} to {executable!r}"
                )
            kernel_comm = (entry / "comm").read_text(encoding="utf-8").strip()
            cmdline = (entry / "cmdline").read_bytes().replace(b"\0", b" ").decode("utf-8", "replace").strip()
            status_text = (entry / "status").read_text(encoding="ascii", errors="replace")
            status_fields = {}
            for line in status_text.splitlines():
                key, sep, value = line.partition(":")
                if sep and key in {"VmRSS", "VmHWM", "VmSize"}:
                    status_fields[key] = int(value.strip().split()[0]) * 1024
            io_fields: dict[str, int] = {}
            for line in (entry / "io").read_text(encoding="ascii", errors="replace").splitlines():
                key, sep, value = line.partition(":")
                if sep and key in {"rchar", "wchar", "read_bytes", "write_bytes"}:
                    io_fields[key] = int(value.strip())
            processes.append(
                {
                    "pid": pid,
                    "kernel_comm": kernel_comm,
                    "executable": executable,
                    "comm": Path(executable.removesuffix(" (deleted)")).name,
                    "cmdline": cmdline,
                    "start_ticks": start_ticks,
                    "cpu_seconds": (int(remainder[11]) + int(remainder[12])) / clock_ticks,
                    "rss_bytes_from_stat": int(remainder[21]) * page_size,
                    "memory_bytes": status_fields,
                    "io_bytes": io_fields,
                }
            )
        except (FileNotFoundError, ProcessLookupError):
            continue
        except PermissionError:
            # A launcher can exit after its identity checks but before /proc/PID/io
            # is read. Only discard the sample if that exact process is now gone.
            try:
                current_stat = (entry / "stat").read_text(encoding="ascii")
                current_fields = current_stat[current_stat.rfind(")") + 2 :].split()
                if int(current_fields[19]) != int(target["start_ticks"]):
                    continue
                os.readlink(entry / "exe")
            except (FileNotFoundError, ProcessLookupError):
                continue
            raise
    return {
        "monotonic_ns": time.monotonic_ns(),
        "wall_time_unix_ns": time.time_ns(),
        "system_cpu_ticks": cpu_ticks,
        "mem_total_bytes": mem_values.get("MemTotal"),
        "mem_available_bytes": mem_values.get("MemAvailable"),
        "processes": sorted(processes, key=lambda item: (item["comm"], item["pid"])),
    }


class ResourceMonitor:
    def __init__(
        self,
        sample_path: Path,
        summary_path: Path,
        targets_path: Path,
        interval_ms: int,
    ):
        self.sample_path = sample_path
        self.summary_path = summary_path
        self.targets_path = targets_path
        self.interval = interval_ms / 1000.0
        self.stop_event = threading.Event()
        self.rows = 0
        self.sampler_error: str | None = None
        self.minimum_available: int | None = None
        self.peak_by_process: dict[str, dict[str, Any]] = {}
        self.maximum_cpu_by_process: dict[str, float] = {}
        self.maximum_io_by_process: dict[str, dict[str, int]] = {}
        self.first_cpu_ticks: int | None = None
        self.last_cpu_ticks: int | None = None
        self.first_monotonic_ns: int | None = None
        self.last_monotonic_ns: int | None = None
        self.stream = sample_path.open("x", encoding="utf-8", buffering=1)
        self.thread = threading.Thread(target=self._run, name="private-resource-monitor", daemon=True)
    def start(self) -> None:
        self.thread.start()

    def stop(self) -> None:
        self.stop_event.set()
        self.thread.join(timeout=max(10.0, self.interval * 5))
        if self.thread.is_alive():
            raise RuntimeError("resource sampler did not stop within its bounded join")
        if self.sampler_error is not None:
            raise RuntimeError(f"resource sampler failed: {self.sampler_error}")

    def _run(self) -> None:
        try:
            while not self.stop_event.is_set():
                row = proc_snapshot(self.targets_path)
                self.rows += 1
                if self.first_cpu_ticks is None:
                    self.first_cpu_ticks = row["system_cpu_ticks"]
                    self.first_monotonic_ns = row["monotonic_ns"]
                self.last_cpu_ticks = row["system_cpu_ticks"]
                self.last_monotonic_ns = row["monotonic_ns"]
                available = row.get("mem_available_bytes")
                if available is not None:
                    self.minimum_available = (
                        available if self.minimum_available is None else min(self.minimum_available, available)
                    )
                for process in row["processes"]:
                    key = f"{process['comm']}:{process['pid']}:{process['start_ticks']}"
                    rss = process["memory_bytes"].get("VmRSS", process["rss_bytes_from_stat"])
                    hwm = process["memory_bytes"].get("VmHWM", rss)
                    peak = self.peak_by_process.setdefault(
                        key,
                        {
                            "comm": process["comm"],
                            "pid": process["pid"],
                            "start_ticks": process["start_ticks"],
                            "peak_rss_bytes": 0,
                            "peak_hwm_bytes": 0,
                            "max_cpu_seconds": 0.0,
                        },
                    )
                    peak["peak_rss_bytes"] = max(peak["peak_rss_bytes"], rss)
                    peak["peak_hwm_bytes"] = max(peak["peak_hwm_bytes"], hwm)
                    peak["max_cpu_seconds"] = max(peak["max_cpu_seconds"], process["cpu_seconds"])
                    self.maximum_cpu_by_process[process["comm"]] = max(
                        self.maximum_cpu_by_process.get(process["comm"], 0.0), process["cpu_seconds"]
                    )
                    io_values = self.maximum_io_by_process.setdefault(process["comm"], {})
                    for counter, value in process["io_bytes"].items():
                        io_values[counter] = max(io_values.get(counter, 0), value)
                self.stream.write(json.dumps(row, sort_keys=True) + "\n")
                self.stream.flush()
                self.stop_event.wait(self.interval)
        except Exception as error:
            self.sampler_error = f"{type(error).__name__}: {error}"
        finally:
            self.stream.close()
            elapsed = (
                (self.last_monotonic_ns - self.first_monotonic_ns) / 1_000_000_000
                if self.first_monotonic_ns is not None and self.last_monotonic_ns is not None
                else 0.0
            )
            json_write(
                self.summary_path,
                {
                    "schema_version": 1,
                    "sample_interval_ms": self.interval * 1000.0,
                    "sample_count": self.rows,
                    "sampled_elapsed_seconds": elapsed,
                    "minimum_mem_available_bytes": self.minimum_available,
                    "system_cpu_tick_delta": (
                        self.last_cpu_ticks - self.first_cpu_ticks
                        if self.first_cpu_ticks is not None and self.last_cpu_ticks is not None
                        else None
                    ),
                    "observed_processes": sorted(
                        self.peak_by_process.values(), key=lambda item: (item["comm"], item["pid"])
                    ),
                    "maximum_cpu_seconds_by_process_name": self.maximum_cpu_by_process,
                    "maximum_io_counters_by_process_name": self.maximum_io_by_process,
                    "sampling_scope": "only explicitly registered installed launcher/search PIDs checked against exact executable paths and /proc start times, plus system memory/CPU counters at the stated interval",
                    "sampler_error": self.sampler_error,
                    "passed": self.sampler_error is None and self.rows > 0,
                },
            )


def run_monitor(arguments: argparse.Namespace) -> None:
    monitor = ResourceMonitor(
        Path(arguments.output_dir) / "resource-samples.jsonl",
        Path(arguments.output_dir) / "resource-summary.json",
        Path(arguments.resource_targets_file),
        arguments.interval_ms,
    )
    stopped = False

    def stop_signal(_signum: int, _frame: Any) -> None:
        nonlocal stopped
        stopped = True
        monitor.stop_event.set()

    signal.signal(signal.SIGTERM, stop_signal)
    signal.signal(signal.SIGINT, stop_signal)
    monitor.start()
    while not stopped:
        time.sleep(0.1)
    monitor.stop()


def import_atspi() -> Any:
    try:
        import gi

        gi.require_version("Atspi", "2.0")
        from gi.repository import Atspi

        Atspi.init()
        return Atspi
    except Exception as error:
        fail(f"Python AT-SPI 2.0 bindings could not initialize: {error}")


def read_name(node: Any) -> str:
    try:
        value = node.get_name()
        return value or ""
    except Exception:
        return ""


def read_role(node: Any, atspi: Any) -> Any:
    try:
        return node.get_role()
    except Exception:
        return None


def read_node_text(node: Any, atspi: Any) -> str:
    try:
        iface = node.get_text_iface()
        if iface is None:
            return ""
        count = atspi.Text.get_character_count(iface)
        return atspi.Text.get_text(iface, 0, count) or ""
    except Exception:
        return ""


def descendants(root: Any, limit: int = 4096):
    pending = [root]
    visited = 0
    while pending and visited < limit:
        node = pending.pop(0)
        visited += 1
        yield node
        try:
            for index in range(node.get_child_count()):
                child = node.get_child_at_index(index)
                if child is not None:
                    pending.append(child)
        except Exception:
            continue


def process_group_alive(process_group_id: int) -> bool:
    try:
        os.killpg(process_group_id, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


class NativeUi:
    def __init__(
        self,
        atspi: Any,
        output_dir: Path,
        process_state_path: Path,
        resource_targets_path: Path,
        timeout_ms: int,
        launcher_binary: str,
    ):
        self.atspi = atspi
        self.output_dir = output_dir
        self.process_state_path = process_state_path
        self.resource_targets_path = resource_targets_path
        self.timeout_ms = timeout_ms
        self.launcher_binary = launcher_binary
        self.launcher: subprocess.Popen[Any] | None = None
        self.window_id: str | None = None
        self.window_pid: int | None = None
        self.last_launch_start_ns: int | None = None
        self.last_query_started_ns: int | None = None
        self.log_stream = (output_dir / "launcher.log").open("a", encoding="utf-8", buffering=1)
        self.accessibility_overhead = "included in keyboard-to-AT-SPI observation; no subtraction is attempted"

    def start(self) -> float:
        if self.launcher is not None and (
            self.launcher.poll() is None or process_group_alive(self.launcher.pid)
        ):
            raise RuntimeError("attempted to start a second resident launcher process group")
        start_ns = time.monotonic_ns()
        self.last_launch_start_ns = start_ns
        self.log_stream.write(f"\n--- launcher start {time.time_ns()} ---\n")
        self.log_stream.flush()
        self.launcher = subprocess.Popen(
            [self.launcher_binary, "--activate"],
            stdin=subprocess.DEVNULL,
            stdout=self.log_stream,
            stderr=subprocess.STDOUT,
            env=os.environ.copy(),
            start_new_session=True,
        )
        self.process_state_path.write_text(f"{self.launcher.pid}\n", encoding="ascii")
        register_resource_process(
            self.resource_targets_path,
            self.launcher.pid,
            self.launcher_binary,
        )
        deadline = time.monotonic() + 15.0
        while time.monotonic() < deadline:
            if self.launcher.poll() is not None:
                raise RuntimeError(f"installed launcher exited early with status {self.launcher.returncode}")
            self.window_id = self._visible_window_id()
            snapshot = self.snapshot()
            if self.window_id is not None and snapshot["window"] is not None and snapshot["query"] is not None:
                self.window_pid = int(
                    run_capture(["xdotool", "getwindowpid", self.window_id], timeout=5.0).strip()
                )
                if self.window_pid != self.launcher.pid:
                    raise RuntimeError(
                        f"visible Slint window PID {self.window_pid} did not match installed launcher PID {self.launcher.pid}"
                    )
                run_capture(["xdotool", "windowactivate", "--sync", self.window_id], timeout=5.0)
                run_capture(["xdotool", "windowfocus", "--sync", self.window_id], timeout=5.0)
                if not self._query_is_focused():
                    raise RuntimeError("native Slint search entry is not AT-SPI focused after launcher activation")
                return (time.monotonic_ns() - start_ns) / 1_000_000.0
            time.sleep(0.05)
        raise TimeoutError("installed native Slint window/query entry did not become AT-SPI visible within 15 seconds")

    def _visible_window_id(self) -> str | None:
        result = subprocess.run(
            ["xdotool", "search", "--onlyvisible", "--name", f"^{WINDOW_TITLE}$"],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=5.0,
        )
        if result.returncode == 1:
            return None
        if result.returncode != 0:
            raise RuntimeError(f"xdotool could not observe private launcher window: {result.stderr.strip()}")
        ids = [line.strip() for line in result.stdout.splitlines() if line.strip()]
        if len(ids) != 1:
            raise RuntimeError(f"expected one private Slint window, found {len(ids)}")
        return ids[0]

    def snapshot(self) -> dict[str, Any]:
        desktop = self.atspi.get_desktop(0)
        nodes = list(descendants(desktop))
        window = next((node for node in nodes if read_name(node) == WINDOW_TITLE), None)
        if window is None:
            return {"window": None, "nodes": [], "query": None, "strings": []}
        window_nodes = list(descendants(window))
        query = next(
            (
                node
                for node in window_nodes
                if read_role(node, self.atspi) == self.atspi.Role.ENTRY
                and read_name(node) == QUERY_LABEL
            ),
            None,
        )
        strings: list[str] = []
        for node in window_nodes:
            name = read_name(node)
            text = read_node_text(node, self.atspi)
            if name:
                strings.append(name)
            if text and text != name:
                strings.append(text)
        return {"window": window, "nodes": window_nodes, "query": query, "strings": strings}

    def buttons(self, snapshot: dict[str, Any] | None = None) -> list[tuple[Any, str]]:
        snapshot = snapshot or self.snapshot()
        return [
            (node, read_name(node))
            for node in snapshot.get("nodes", [])
            if read_role(node, self.atspi) == self.atspi.Role.PUSH_BUTTON and read_name(node)
        ]

    def _query_is_focused(self) -> bool:
        snapshot = self.snapshot()
        query = snapshot.get("query")
        if query is None:
            return False
        try:
            return query.get_state_set().contains(self.atspi.StateType.FOCUSED)
        except Exception:
            return False

    def search_text(self) -> str:
        snapshot = self.snapshot()
        query = snapshot.get("query")
        if query is None:
            return ""
        return read_node_text(query, self.atspi)

    def type_query(
        self,
        query: str,
        *,
        expected_path: str | None = None,
        expected_result_label: str | None = None,
        expect_no_match: bool = False,
    ) -> dict[str, Any]:
        if self.window_id is None or self.launcher is None or self.launcher.poll() is not None:
            raise RuntimeError("native launcher is not live before query input")
        run_capture(["xdotool", "windowfocus", "--sync", self.window_id], timeout=5.0)
        query_started_ns = time.monotonic_ns()
        self.last_query_started_ns = query_started_ns
        run_capture(["xdotool", "key", "--clearmodifiers", "ctrl+a"], timeout=5.0)
        run_capture(["xdotool", "type", "--clearmodifiers", "--delay", "0", query], timeout=5.0)
        deadline = time.monotonic() + self.timeout_ms / 1000.0
        last_snapshot: dict[str, Any] | None = None
        while time.monotonic() < deadline:
            if self.launcher.poll() is not None:
                raise RuntimeError(f"installed launcher exited during query with status {self.launcher.returncode}")
            last_snapshot = self.snapshot()
            actual_query = read_node_text(last_snapshot["query"], self.atspi) if last_snapshot.get("query") else ""
            if actual_query == query:
                button_rows = self.buttons(last_snapshot)
                labels = [name for _node, name in button_rows]
                if expected_result_label is not None:
                    matches = [
                        (node, name)
                        for node, name in button_rows
                        if expected_result_label in name
                    ]
                    if matches:
                        return {
                            "kind": "result",
                            "query_started_ns": query_started_ns,
                            "observed_ns": time.monotonic_ns(),
                            "latency_ms": (time.monotonic_ns() - query_started_ns) / 1_000_000.0,
                            "result_label": matches[0][1],
                            "result_role": str(read_role(matches[0][0], self.atspi)),
                            "diagnostic_labels": labels,
                        }
                elif expected_path is not None:
                    matches = [
                        (node, name)
                        for node, name in button_rows
                        if expected_path in name and query in name
                    ]
                    if matches:
                        return {
                            "kind": "result",
                            "query_started_ns": query_started_ns,
                            "observed_ns": time.monotonic_ns(),
                            "latency_ms": (time.monotonic_ns() - query_started_ns) / 1_000_000.0,
                            "result_label": matches[0][1],
                            "result_role": str(read_role(matches[0][0], self.atspi)),
                            "diagnostic_labels": labels,
                        }
                if expect_no_match and any(
                    "No matching file paths or cited passages." in text
                    for text in last_snapshot["strings"]
                ):
                    return {
                        "kind": "no_match",
                        "query_started_ns": query_started_ns,
                        "observed_ns": time.monotonic_ns(),
                        "latency_ms": (time.monotonic_ns() - query_started_ns) / 1_000_000.0,
                        "result_label": "No matching file paths or cited passages.",
                        "diagnostic_labels": labels,
                    }
            time.sleep(0.02)
        labels = self.buttons(last_snapshot) if last_snapshot else []
        strings = last_snapshot.get("strings", []) if last_snapshot else []
        raise TimeoutError(
            f"AT-SPI query observation timed out after {self.timeout_ms}ms; "
            f"query={query!r}, actual={self.search_text()!r}, "
            f"expected_path={expected_path!r}, expected_label={expected_result_label!r}, "
            f"no_match_expected={expect_no_match}, buttons={[name for _node, name in labels]}, "
            f"status_strings={strings[-12:]}"
        )

    def open_selected_result(self, expected_path: str, expected_query: str) -> float:
        snapshot = self.snapshot()
        result_list = next(
            (
                node
                for node in snapshot["nodes"]
                if read_role(node, self.atspi) == self.atspi.Role.LIST_BOX
                and read_name(node) == "Launcher results"
            ),
            None,
        )
        if result_list is None:
            raise RuntimeError("native launcher result list was not present in the AT-SPI tree")
        result_rows = [
            (node, read_name(node))
            for node in descendants(result_list)
            if read_role(node, self.atspi) == self.atspi.Role.PUSH_BUTTON and read_name(node)
        ]
        target_index = next(
            (
                index
                for index, (_node, name) in enumerate(result_rows)
                if expected_path in name and expected_query in name
            ),
            None,
        )
        if target_index is None:
            raise RuntimeError(
                f"no accessible passage result matched path {expected_path!r} and query {expected_query!r}"
            )
        for _index in range(target_index):
            run_capture(["xdotool", "key", "--clearmodifiers", "Down"], timeout=5.0)
            time.sleep(0.02)
        before = time.monotonic_ns()
        run_capture(["xdotool", "key", "--clearmodifiers", "Return"], timeout=5.0)
        deadline = time.monotonic() + 5.0
        while time.monotonic() < deadline:
            current = self.snapshot()
            visible_buttons = {
                name for node, name in self.buttons(current)
                if node.get_state_set().contains(self.atspi.StateType.SHOWING)
            }
            detail_rows = [
                read_name(node)
                for node in current["nodes"]
                if read_role(node, self.atspi) in (self.atspi.Role.GROUPING, self.atspi.Role.PANEL)
                and node.get_state_set().contains(self.atspi.StateType.SHOWING)
                and read_name(node).startswith("Passage ")
                and expected_path in read_name(node)
            ]
            if COPY_CITATION_LABEL in visible_buttons and COPY_EXCERPT_LABEL in visible_buttons and detail_rows:
                return (time.monotonic_ns() - before) / 1_000_000.0
        raise TimeoutError(f"keyboard Return did not open a cited passage detail for {expected_path}")

    def return_to_results(self) -> None:
        snapshot = self.snapshot()
        label = "Return to launcher results"
        if label in {name for _node, name in self.buttons(snapshot)}:
            self.invoke_button(label)
        elif self._query_is_focused():
            return
        else:
            raise RuntimeError("native search entry is unfocused and the accessible Return to results control is absent")
        deadline = time.monotonic() + 5.0
        while time.monotonic() < deadline:
            snapshot = self.snapshot()
            names = {name for _node, name in self.buttons(snapshot)}
            if label not in names and snapshot.get("query") is not None and self._query_is_focused():
                return
            time.sleep(0.02)
        raise TimeoutError("AT-SPI Return to results did not restore focus to the native launcher search entry")

    def invoke_button(self, label: str) -> None:
        snapshot = self.snapshot()
        button = next((node for node, name in self.buttons(snapshot) if name == label), None)
        if button is None:
            raise RuntimeError(f"AT-SPI button not available: {label}")
        action = button.get_action_iface()
        if action is None or not self.atspi.Action.do_action(action, 0):
            raise RuntimeError(f"AT-SPI could not invoke native button: {label}")

    def wait_for_strings(self, expected: tuple[str, ...], timeout_seconds: float = 8.0) -> tuple[str, float]:
        started = time.monotonic_ns()
        deadline = time.monotonic() + timeout_seconds
        last_strings: list[str] = []
        while time.monotonic() < deadline:
            snapshot = self.snapshot()
            last_strings = snapshot["strings"]
            joined = "\n".join(last_strings)
            for text in expected:
                if text in joined:
                    return text, (time.monotonic_ns() - started) / 1_000_000.0
            time.sleep(0.02)
        raise TimeoutError(f"native UI did not expose any expected text {expected!r}; last={last_strings[-12:]}")

    def clipboard(self) -> str:
        return run_capture(["xclip", "-selection", "clipboard", "-o"], timeout=3.0)

    def copy_fresh_citation_and_excerpt(
        self,
        expected_path: str,
        expected_citation: str,
        expected_excerpt_token: str,
        forbidden_excerpt_token: str | None = None,
    ) -> dict[str, Any]:
        started = time.monotonic_ns()
        self.invoke_button(COPY_CITATION_LABEL)
        citation_notice, _ = self.wait_for_strings(("Copied citation.",), timeout_seconds=8.0)
        citation_clipboard = self.clipboard()
        if citation_clipboard != expected_citation:
            raise RuntimeError(
                f"fresh authorized citation mismatch for {expected_path}: "
                f"expected {expected_citation!r}, received {citation_clipboard!r}"
            )
        self.invoke_button(COPY_EXCERPT_LABEL)
        excerpt_notice, _ = self.wait_for_strings(("Copied reopened passage.",), timeout_seconds=8.0)
        excerpt_clipboard = self.clipboard()
        if expected_excerpt_token not in excerpt_clipboard:
            raise RuntimeError(
                f"fresh authorized excerpt omitted expected source text {expected_excerpt_token!r}: "
                f"{excerpt_clipboard!r}"
            )
        if forbidden_excerpt_token and forbidden_excerpt_token in excerpt_clipboard:
            raise RuntimeError(
                f"fresh authorized excerpt still contained replaced source text {forbidden_excerpt_token!r}"
            )
        return {
            "citation": citation_clipboard,
            "excerpt": excerpt_clipboard,
            "citation_notice": citation_notice,
            "excerpt_notice": excerpt_notice,
            "fresh_reopen_ms": (time.monotonic_ns() - started) / 1_000_000.0,
        }

    def verify_stale_evidence_denied(self, prior_clipboard: str) -> dict[str, Any]:
        started = time.monotonic_ns()
        self.invoke_button(COPY_CITATION_LABEL)
        message, elapsed = self.wait_for_strings(REOPEN_DENIALS, timeout_seconds=8.0)
        clipboard = self.clipboard()
        if clipboard != prior_clipboard:
            raise RuntimeError(
                f"stale evidence action changed the private clipboard despite revalidation failure: {clipboard!r}"
            )
        response = search_response(
            os.environ["MAESTRIA_SEARCH_BINARY"],
            "status",
            os.environ["MAESTRIA_SEARCH_SOCKET"],
            os.environ["MAESTRIA_SEARCH_REALM"],
            os.environ["MAESTRIA_SEARCH_CREDENTIAL"],
        )
        if response.get("type") != "status" or not isinstance(response.get("data"), dict):
            raise RuntimeError("search daemon status did not remain available after stale-evidence denial")
        return {
            "denial_message": message,
            "denial_ms": elapsed,
            "mutation_to_denial_ms": (time.monotonic_ns() - started) / 1_000_000.0,
            "clipboard_unchanged": True,
            "authorized_daemon_status_after_denial": response,
        }

    def close(self) -> None:
        process = self.launcher
        if process is None:
            return
        if process.poll() is None:
            try:
                subprocess.run(
                    [self.launcher_binary, "--quit"],
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    env=os.environ.copy(),
                    check=False,
                    timeout=10.0,
                )
            except Exception as error:
                self.log_stream.write(f"launcher graceful quit failed: {error}\n")
            try:
                process.wait(timeout=5.0)
            except subprocess.TimeoutExpired:
                pass
        if process_group_alive(process.pid):
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            deadline = time.monotonic() + 3.0
            while process_group_alive(process.pid) and time.monotonic() < deadline:
                time.sleep(0.05)
        if process_group_alive(process.pid):
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            deadline = time.monotonic() + 3.0
            while process_group_alive(process.pid) and time.monotonic() < deadline:
                time.sleep(0.05)
        if process.poll() is None:
            try:
                process.wait(timeout=1.0)
            except subprocess.TimeoutExpired:
                pass
        if process.poll() is None or process_group_alive(process.pid):
            self.log_stream.write("launcher process group survived TERM and KILL\n")
            return
        self.process_state_path.unlink(missing_ok=True)
        self.launcher = None
        self.window_id = None
        self.window_pid = None


def make_observation(kind: str, index: int, sample: dict[str, Any]) -> dict[str, Any]:
    return {
        "class": kind,
        "sample_index": index,
        "query": sample.get("query"),
        "old_query": sample.get("old_query"),
        "approved_source_path": sample.get("path"),
        "expected_citation": sample.get("expected_citation"),
        "sample_timeout_ms": None,
        "outcome": "not_started",
        "error": None,
        "query_input_attempted": False,
        "precondition_query_attempted": False,
        "fresh_query_attempted": False,
        "old_query_absence_attempted": False,
        "query_to_result_ms": None,
        "latency_ms_including_failures": None,
        "latency_observation_scope": None,
        "latency_censored_at_timeout": False,
        "result_to_reopen_ms": None,
        "fresh_reopen_ms": None,
        "source_change": None,
        "source_change_durable_verified": False,
        "source_change_to_reauthorization_ms": None,
        "source_change_to_fresh_result_ms": None,
        "source_change_to_fresh_citation_ms": None,
        "old_query_absence_ms": None,
        "precondition_query_ms": None,
        "precondition_citation": None,
        "precondition_excerpt": None,
        "stale_reauthorization": None,
        "old_query_absence_censored_at_timeout": False,
        "launcher_start_to_ready_ms": None,
        "launcher_start_to_result_ms": None,
        "launcher_start_to_result_or_failure_ms": None,
        "launcher_start_failure_ms": None,
        "result_observed": False,
        "fresh_citation_verified": False,
        "fresh_citation": None,
        "fresh_excerpt_verified": False,
        "fresh_excerpt": None,
        "stale_evidence_denied": False,
        "post_change_old_query_absent": None,
        "atspi_result_label": None,
        "atspi_error_text": None,
        "query_latency_le_100ms": None,
        "measurement_scope": "private installed native Slint keyboard input to first matching AT-SPI result; AT-SPI polling and observer overhead are included",
    }


def percentile_summary(values: list[float]) -> dict[str, Any]:
    ordered = sorted(values)
    if not ordered:
        return {"count": 0, "p50_ms": None, "p95_ms": None, "p99_ms": None, "max_ms": None}

    def nearest_rank(percent: float) -> float:
        rank = max(1, math.ceil(percent * len(ordered)))
        return ordered[rank - 1]

    return {
        "count": len(ordered),
        "p50_ms": nearest_rank(0.50),
        "p95_ms": nearest_rank(0.95),
        "p99_ms": nearest_rank(0.99),
        "max_ms": ordered[-1],
    }


def child_process_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
def installed_daemon_process(binary: str, pid: int, socket_path: str) -> dict[str, Any]:
    report: dict[str, Any] = {
        "pid": pid,
        "expected_binary": str(Path(binary).resolve(strict=True)),
        "socket_path": socket_path,
        "passed": False,
    }
    try:
        if not child_process_alive(pid):
            raise RuntimeError("private installed search daemon process is not alive")
        stat = Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
        if stat[stat.rfind(")") + 2 :].split()[0] in {"Z", "X"}:
            raise RuntimeError("private installed search daemon is a zombie/exited process")
        report["executable"] = os.path.realpath(f"/proc/{pid}/exe")
        argv = Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")
        arguments = [part.decode("utf-8", "replace") for part in argv if part]
        report["argv"] = arguments
        if report["executable"] != report["expected_binary"]:
            raise RuntimeError(f"daemon executable mismatch: {report['executable']}")
        instance_dir = str(Path(socket_path).resolve().parent.parent)
        if "start" not in arguments or "--instance-dir" not in arguments or instance_dir not in arguments:
            raise RuntimeError("daemon argv did not bind the private instance directory")
        report["private_instance_directory"] = instance_dir
        report["passed"] = True
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    return report


def capture_window_evidence(driver: NativeUi, output_dir: Path, record: dict[str, Any]) -> None:
    if not driver.window_id:
        raise RuntimeError("no private Slint window is available for visual evidence")
    screenshot = output_dir / "native-cold-first-result.xwd"
    if screenshot.exists():
        return
    run_capture(
        ["xwd", "-silent", "-display", os.environ["DISPLAY"], "-id", driver.window_id, "-out", str(screenshot)],
        timeout=10.0,
    )
    record["screenshot_path"] = str(screenshot)
    record["screenshot_format"] = "XWD captured from the private Xvfb window, not the host desktop"


def record_observation(path: Path, observations: list[dict[str, Any]], record: dict[str, Any]) -> None:
    observations.append(record)
    jsonl_append(path, record)


def _elapsed_ms(start_ns: int | None, end_ns: int | None = None) -> float | None:
    if start_ns is None:
        return None
    if end_ns is None:
        end_ns = time.monotonic_ns()
    return max(0.0, (end_ns - start_ns) / 1_000_000.0)
def is_timeout_error(error: BaseException) -> bool:
    return isinstance(error, (TimeoutError, subprocess.TimeoutExpired))



def drive(arguments: argparse.Namespace) -> None:
    output_dir = Path(arguments.output_dir).resolve(strict=True)
    fixture = json.loads(Path(arguments.manifest).read_text(encoding="utf-8"))
    if fixture.get("approved_text_file_count") != TEXT_FILE_COUNT or fixture.get("desktop_entry_count") != DESKTOP_ENTRY_COUNT:
        fail("fixture manifest does not describe exactly 10,000 approved text files and 500 private desktop entries")
    if fixture.get("approved_text_bytes", TEXT_FILE_CAP_BYTES + 1) > TEXT_FILE_CAP_BYTES:
        fail("approved corpus exceeds its declared 100 MiB cap")
    for kind in ("cold", "active", "edit", "delete"):
        if len(fixture.get("classes", {}).get(kind, [])) != SAMPLES_PER_CLASS:
            fail(f"fixture does not contain exactly {SAMPLES_PER_CLASS} {kind} observations")

    atspi = import_atspi()
    process_state_path = Path(arguments.process_state)
    os.environ["MAESTRIA_SEARCH_BINARY"] = arguments.search_binary
    os.environ["MAESTRIA_SEARCH_SOCKET"] = arguments.socket_path
    os.environ["MAESTRIA_SEARCH_REALM"] = arguments.consumer_realm
    os.environ["MAESTRIA_SEARCH_CREDENTIAL"] = arguments.credential_file
    daemon_report = installed_daemon_process(arguments.search_binary, arguments.daemon_pid, arguments.socket_path)
    json_write(output_dir / "daemon-process.json", daemon_report)
    driver = NativeUi(
        atspi,
        output_dir,
        process_state_path,
        Path(arguments.resource_targets_file),
        arguments.sample_timeout_ms,
        arguments.launcher_binary,
    )
    observations_path = output_dir / "observations.jsonl"
    if observations_path.exists():
        fail(f"refusing to overwrite existing observations: {observations_path}")
    observations: list[dict[str, Any]] = []
    class_records: dict[str, list[dict[str, Any]]] = defaultdict(list)
    desktop_validation: dict[str, Any] = {
        "desktop_entry_count": DESKTOP_ENTRY_COUNT,
        "query": "Native Benchmark Application 499",
        "passed": False,
        "error": None,
    }
    ui_evidence: dict[str, Any] = {
        "window_name": WINDOW_TITLE,
        "query_entry_accessible_name": QUERY_LABEL,
        "query_entry_role": "entry",
        "display_backend": "private X11/Xvfb; WAYLAND_DISPLAY unset",
        "keyboard_input": "xdotool types queries into the native Slint entry, selects cited results with Down, and opens them with Return",
        "result_observation": "AT-SPI 2.0 accessible result labels, detailed citation/excerpt controls, and private clipboard copy actions",
        "screenshot_path": None,
        "atspi_initialized": True,
        "latency_observer_overhead": driver.accessibility_overhead,
        "installed_launcher_binary": arguments.launcher_binary,
        "search_daemon_process": daemon_report,
    }
    benchmark_start_ns = time.monotonic_ns()
    try:
        # Confirm the launcher actually loaded all private desktop entries before measuring.
        try:
            startup_ms = driver.start()
            desktop = driver.type_query(
                "Native Benchmark Application 499",
                expected_result_label="Native Benchmark Application 499",
            )
            desktop_validation.update(
                {
                    "passed": True,
                    "launcher_start_to_ready_ms": startup_ms,
                    "query_to_atspi_application_ms": desktop["latency_ms"],
                    "atspi_result_label": desktop["result_label"],
                    "desktop_file": str(Path(fixture["desktop_entry_directory"]) / "native-benchmark-499.desktop"),
                }
            )
        except Exception as error:
            desktop_validation["error"] = f"{type(error).__name__}: {error}"
        finally:
            driver.close()

        # Cold means a newly started installed UI process and an unseen corpus query,
        # not a claim that kernel page caches or the already-indexed daemon are cold.
        for sample in fixture["classes"]["cold"]:
            record = make_observation("cold", sample["sample_index"], sample)
            record["sample_timeout_ms"] = arguments.sample_timeout_ms
            sample_started_ns = time.monotonic_ns()
            launcher_start_ns: int | None = None
            query_started_ns: int | None = None
            driver.last_launch_start_ns = None
            driver.last_query_started_ns = None
            try:
                launch_ms = driver.start()
                launcher_start_ns = driver.last_launch_start_ns
                record["launcher_start_to_ready_ms"] = launch_ms
                result = driver.type_query(sample["query"], expected_path=sample["path"])
                query_started_ns = result["query_started_ns"]
                record["query_to_result_ms"] = result["latency_ms"]
                record["launcher_start_to_result_ms"] = (
                    result["observed_ns"] - launcher_start_ns
                ) / 1_000_000.0 if launcher_start_ns is not None else None
                record["result_observed"] = True
                record["atspi_result_label"] = result["result_label"]
                record["query_latency_le_100ms"] = result["latency_ms"] <= INTERNAL_DEADLINE_MS
                record["result_to_reopen_ms"] = driver.open_selected_result(sample["path"], sample["query"])
                fresh = driver.copy_fresh_citation_and_excerpt(
                    sample["path"], sample["expected_citation"], sample["query"]
                )
                record["fresh_reopen_ms"] = fresh["fresh_reopen_ms"]
                record["fresh_citation"] = fresh["citation"]
                record["fresh_excerpt"] = fresh["excerpt"]
                record["fresh_citation_verified"] = fresh["citation"] == sample["expected_citation"]
                record["fresh_excerpt_verified"] = sample["query"] in fresh["excerpt"]
                record["outcome"] = "passed" if record["fresh_citation_verified"] and record["fresh_excerpt_verified"] else "failed"
                if not (output_dir / "native-cold-first-result.xwd").exists():
                    capture_window_evidence(driver, output_dir, ui_evidence)
                driver.return_to_results()
            except Exception as error:
                record["outcome"] = "timeout" if is_timeout_error(error) else "failed"
                record["error"] = f"{type(error).__name__}: {error}"
                record["atspi_error_text"] = str(error)
                if query_started_ns is None and driver.last_query_started_ns is not None:
                    query_started_ns = driver.last_query_started_ns
            finally:
                if launcher_start_ns is None:
                    launcher_start_ns = driver.last_launch_start_ns
                if record["launcher_start_to_ready_ms"] is None and launcher_start_ns is not None:
                    record["launcher_start_to_ready_ms"] = _elapsed_ms(launcher_start_ns)
                if launcher_start_ns is not None:
                    record["launcher_start_to_result_or_failure_ms"] = _elapsed_ms(launcher_start_ns)
                if query_started_ns is None and driver.last_query_started_ns is not None:
                    query_started_ns = driver.last_query_started_ns
                record["query_input_attempted"] = query_started_ns is not None
                if record["query_to_result_ms"] is None and query_started_ns is not None:
                    record["query_to_result_ms"] = _elapsed_ms(query_started_ns)
                    record["query_latency_le_100ms"] = (
                        record["query_to_result_ms"] <= INTERNAL_DEADLINE_MS
                    )
                record["latency_ms_including_failures"] = (
                    record["query_to_result_ms"]
                    if record["query_to_result_ms"] is not None
                    else _elapsed_ms(launcher_start_ns or sample_started_ns)
                )
                record["latency_observation_scope"] = (
                    "keyboard input to first matching AT-SPI result"
                    if query_started_ns is not None
                    else "launcher process launch/readiness through a failure before query input"
                )
                record["latency_censored_at_timeout"] = (
                    record["outcome"] == "timeout" and not record["result_observed"]
                )
                record_observation(observations_path, observations, record)
                class_records["cold"].append(record)
                driver.close()

        # Active samples share one already-visible native process; startup readiness
        # is recorded separately from keyboard-to-result latency.
        active_session_ready_ms: float | None = None
        active_session_error: str | None = None
        active_session_timed_out = False
        active_session_failure_ms: float | None = None
        driver.last_launch_start_ns = None
        try:
            active_session_ready_ms = driver.start()
        except Exception as error:
            active_session_error = f"{type(error).__name__}: {error}"
            active_session_timed_out = is_timeout_error(error)
            active_session_failure_ms = _elapsed_ms(driver.last_launch_start_ns)
        for sample in fixture["classes"]["active"]:
            record = make_observation("active", sample["sample_index"], sample)
            record["sample_timeout_ms"] = arguments.sample_timeout_ms
            record["launcher_start_to_ready_ms"] = active_session_ready_ms
            record["launcher_start_failure_ms"] = active_session_failure_ms
            started_ns = time.monotonic_ns()
            query_started_ns: int | None = None
            driver.last_query_started_ns = None
            try:
                if active_session_error:
                    message = f"active launcher startup failed: {active_session_error}"
                    if active_session_timed_out:
                        raise TimeoutError(message)
                    raise RuntimeError(message)
                result = driver.type_query(sample["query"], expected_path=sample["path"])
                query_started_ns = result["query_started_ns"]
                record["query_to_result_ms"] = result["latency_ms"]
                record["result_observed"] = True
                record["atspi_result_label"] = result["result_label"]
                record["query_latency_le_100ms"] = result["latency_ms"] <= INTERNAL_DEADLINE_MS
                record["result_to_reopen_ms"] = driver.open_selected_result(sample["path"], sample["query"])
                fresh = driver.copy_fresh_citation_and_excerpt(
                    sample["path"], sample["expected_citation"], sample["query"]
                )
                record["fresh_reopen_ms"] = fresh["fresh_reopen_ms"]
                record["fresh_citation"] = fresh["citation"]
                record["fresh_excerpt"] = fresh["excerpt"]
                record["fresh_citation_verified"] = fresh["citation"] == sample["expected_citation"]
                record["fresh_excerpt_verified"] = sample["query"] in fresh["excerpt"]
                record["outcome"] = "passed" if record["fresh_citation_verified"] and record["fresh_excerpt_verified"] else "failed"
                driver.return_to_results()
            except Exception as error:
                record["outcome"] = "timeout" if is_timeout_error(error) else "failed"
                record["error"] = f"{type(error).__name__}: {error}"
                record["atspi_error_text"] = str(error)
                if query_started_ns is None and driver.last_query_started_ns is not None:
                    query_started_ns = driver.last_query_started_ns
            finally:
                if query_started_ns is None and driver.last_query_started_ns is not None:
                    query_started_ns = driver.last_query_started_ns
                record["query_input_attempted"] = query_started_ns is not None
                if record["query_to_result_ms"] is None and query_started_ns is not None:
                    record["query_to_result_ms"] = _elapsed_ms(query_started_ns)
                    record["query_latency_le_100ms"] = (
                        record["query_to_result_ms"] <= INTERNAL_DEADLINE_MS
                    )
                record["latency_ms_including_failures"] = (
                    record["query_to_result_ms"]
                    if record["query_to_result_ms"] is not None
                    else (
                        active_session_failure_ms
                        if active_session_error is not None
                        else _elapsed_ms(started_ns)
                    )
                )
                record["latency_observation_scope"] = (
                    "keyboard input to first matching AT-SPI result"
                    if query_started_ns is not None
                    else (
                        "active launcher startup attempt to its recorded failure"
                        if active_session_error is not None
                        else "active launcher session failure before query input"
                    )
                )
                record["latency_censored_at_timeout"] = (
                    record["outcome"] == "timeout" and not record["result_observed"]
                )
                record_observation(observations_path, observations, record)
                class_records["active"].append(record)

        driver.close()

        # Edited evidence is first present in the UI, then replaced on disk. The
        # old evidence ID must be freshly denied; a new search must cite and reopen
        # only the changed bytes, and the old term must disappear afterward.
        edit_session_error: str | None = None
        edit_session_timed_out = False
        edit_session_failure_ms: float | None = None
        driver.last_launch_start_ns = None
        try:
            driver.start()
        except Exception as error:
            edit_session_error = f"{type(error).__name__}: {error}"
            edit_session_timed_out = is_timeout_error(error)
            edit_session_failure_ms = _elapsed_ms(driver.last_launch_start_ns)
        for sample in fixture["classes"]["edit"]:
            record = make_observation("edit", sample["sample_index"], sample)
            record["sample_timeout_ms"] = arguments.sample_timeout_ms
            record["launcher_start_failure_ms"] = edit_session_failure_ms
            sample_started_ns = time.monotonic_ns()
            mutation_ns: int | None = None
            precondition_started_ns: int | None = None
            query_started_ns: int | None = None
            absence_started_ns: int | None = None
            old_clipboard: str | None = None
            path = Path(sample["path"])
            driver.last_query_started_ns = None
            try:
                if edit_session_error:
                    message = f"edit launcher startup failed: {edit_session_error}"
                    if edit_session_timed_out:
                        raise TimeoutError(message)
                    raise RuntimeError(message)
                driver.last_query_started_ns = None
                record["precondition_query_attempted"] = True
                try:
                    precondition = driver.type_query(sample["old_query"], expected_path=sample["path"])
                except Exception:
                    precondition_started_ns = driver.last_query_started_ns
                    raise
                precondition_started_ns = precondition["query_started_ns"]
                record["precondition_query_ms"] = precondition["latency_ms"]
                driver.open_selected_result(sample["path"], sample["old_query"])
                old = driver.copy_fresh_citation_and_excerpt(
                    sample["path"], sample["expected_citation"], sample["old_query"]
                )
                record["precondition_citation"] = old["citation"]
                record["precondition_excerpt"] = old["excerpt"]
                if old["citation"] != sample["expected_citation"] or sample["old_query"] not in old["excerpt"]:
                    raise RuntimeError("pre-edit UI result was not freshly authorized with its original cited passage")
                old_clipboard = old["citation"]
                mutation_ns = time.monotonic_ns()
                content = (sample["expected_content"] + "\n").encode("utf-8")
                record["source_change"] = {
                    "operation": "replace",
                    "path": str(path),
                    "previous_sha256": sample["initial_sha256"],
                    "expected_sha256": hashlib.sha256(content).hexdigest(),
                    "expected_size_bytes": len(content),
                    "write_fsynced": False,
                    "directory_fsynced": False,
                }
                with path.open("wb") as stream:
                    stream.write(content)
                    stream.flush()
                    os.fsync(stream.fileno())
                record["source_change"]["write_fsynced"] = True
                directory_fd = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
                try:
                    os.fsync(directory_fd)
                    record["source_change"]["directory_fsynced"] = True
                finally:
                    os.close(directory_fd)
                stale = driver.verify_stale_evidence_denied(old_clipboard)
                record["stale_evidence_denied"] = stale["clipboard_unchanged"]
                record["stale_reauthorization"] = {
                    "message": stale["denial_message"],
                    "clipboard_unchanged": stale["clipboard_unchanged"],
                    "mutation_to_denial_ms": stale["mutation_to_denial_ms"],
                    "authorized_daemon_status_after_denial": stale["authorized_daemon_status_after_denial"],
                }
                record["source_change_to_reauthorization_ms"] = (
                    time.monotonic_ns() - mutation_ns
                ) / 1_000_000.0
                record["atspi_error_text"] = stale["denial_message"]
                driver.return_to_results()
                driver.last_query_started_ns = None
                record["fresh_query_attempted"] = True
                result = driver.type_query(sample["query"], expected_path=sample["path"])
                query_started_ns = result["query_started_ns"]
                record["query_to_result_ms"] = result["latency_ms"]
                record["source_change_to_fresh_result_ms"] = (
                    result["observed_ns"] - mutation_ns
                ) / 1_000_000.0
                record["result_observed"] = True
                record["atspi_result_label"] = result["result_label"]
                record["query_latency_le_100ms"] = result["latency_ms"] <= INTERNAL_DEADLINE_MS
                record["result_to_reopen_ms"] = driver.open_selected_result(sample["path"], sample["query"])
                fresh = driver.copy_fresh_citation_and_excerpt(
                    sample["path"], sample["expected_citation"], sample["query"], sample["old_query"]
                )
                record["fresh_reopen_ms"] = fresh["fresh_reopen_ms"]
                record["fresh_citation"] = fresh["citation"]
                record["fresh_excerpt"] = fresh["excerpt"]
                record["source_change_to_fresh_citation_ms"] = (
                    time.monotonic_ns() - mutation_ns
                ) / 1_000_000.0
                record["fresh_citation_verified"] = fresh["citation"] == sample["expected_citation"]
                record["fresh_excerpt_verified"] = (
                    sample["query"] in fresh["excerpt"] and sample["old_query"] not in fresh["excerpt"]
                )
                driver.return_to_results()
                driver.last_query_started_ns = None
                record["old_query_absence_attempted"] = True
                try:
                    absent = driver.type_query(sample["old_query"], expect_no_match=True)
                except Exception:
                    absence_started_ns = driver.last_query_started_ns
                    raise
                absence_started_ns = absent["query_started_ns"]
                record["old_query_absence_ms"] = absent["latency_ms"]
                record["post_change_old_query_absent"] = absent["kind"] == "no_match"
                record["outcome"] = (
                    "passed"
                    if record["stale_evidence_denied"]
                    and record["fresh_citation_verified"]
                    and record["fresh_excerpt_verified"]
                    and record["post_change_old_query_absent"]
                    else "failed"
                )
                driver.return_to_results()
            except Exception as error:
                record["outcome"] = "timeout" if is_timeout_error(error) else "failed"
                record["error"] = f"{type(error).__name__}: {error}"
                record["atspi_error_text"] = str(error)
                if record["fresh_query_attempted"] and query_started_ns is None:
                    query_started_ns = driver.last_query_started_ns
                if record["old_query_absence_attempted"] and absence_started_ns is None:
                    absence_started_ns = driver.last_query_started_ns
            finally:
                if (
                    precondition_started_ns is None
                    and record["precondition_query_attempted"]
                    and not record["fresh_query_attempted"]
                ):
                    precondition_started_ns = driver.last_query_started_ns
                if record["precondition_query_ms"] is None and precondition_started_ns is not None:
                    record["precondition_query_ms"] = _elapsed_ms(precondition_started_ns)
                if query_started_ns is None and record["fresh_query_attempted"]:
                    query_started_ns = driver.last_query_started_ns
                if record["fresh_query_attempted"] and query_started_ns is not None:
                    record["query_input_attempted"] = True
                    if record["query_to_result_ms"] is None:
                        record["query_to_result_ms"] = _elapsed_ms(query_started_ns)
                        record["query_latency_le_100ms"] = (
                            record["query_to_result_ms"] <= INTERNAL_DEADLINE_MS
                        )
                if record["old_query_absence_attempted"] and absence_started_ns is not None:
                    if record["old_query_absence_ms"] is None:
                        record["old_query_absence_ms"] = _elapsed_ms(absence_started_ns)
                    record["old_query_absence_censored_at_timeout"] = record["outcome"] == "timeout"
                record["query_input_attempted"] = any(
                    started is not None
                    for started in (precondition_started_ns, query_started_ns, absence_started_ns)
                )
                if record["source_change"] is not None:
                    try:
                        exists = path.exists() or path.is_symlink()
                        record["source_change"]["actual_file_exists"] = exists
                        if path.is_file() and not path.is_symlink():
                            actual = path.read_bytes()
                            record["source_change"]["actual_size_bytes"] = len(actual)
                            record["source_change"]["actual_sha256"] = hashlib.sha256(actual).hexdigest()
                    except Exception as inspection_error:
                        record["source_change"]["final_state_error"] = (
                            f"{type(inspection_error).__name__}: {inspection_error}"
                        )
                source_change = record["source_change"]
                record["source_change_durable_verified"] = bool(
                    source_change is not None
                    and source_change.get("write_fsynced") is True
                    and source_change.get("directory_fsynced") is True
                    and source_change.get("actual_file_exists") is True
                    and source_change.get("actual_sha256") == source_change.get("expected_sha256")
                )
                if record["outcome"] == "passed" and not record["source_change_durable_verified"]:
                    record["outcome"] = "failed"
                    record["error"] = "edited fixture bytes or filesystem durability did not match the expected source change"
                record["latency_ms_including_failures"] = (
                    record["source_change_to_fresh_result_ms"]
                    if record["source_change_to_fresh_result_ms"] is not None
                    else (
                        edit_session_failure_ms
                        if edit_session_error is not None
                        else _elapsed_ms(mutation_ns or sample_started_ns)
                    )
                )
                record["latency_observation_scope"] = (
                    "durable source replacement to first matching AT-SPI result"
                    if mutation_ns is not None
                    else (
                        "edit launcher startup attempt to its recorded failure"
                        if edit_session_error is not None
                        else "sample start through a failure before the durable source change"
                    )
                )
                record["latency_censored_at_timeout"] = (
                    record["outcome"] == "timeout" and not record["result_observed"]
                )
                record_observation(observations_path, observations, record)
                class_records["edit"].append(record)
                if driver.launcher is not None and driver.launcher.poll() is None:
                    try:
                        driver.return_to_results()
                    except Exception:
                        pass
        driver.close()

        # Deletion retains the old passage view long enough to force a new
        # authenticated evidence open after unlink, then proves fresh search has
        # no citation. No source-opening application is invoked.
        delete_session_error: str | None = None
        delete_session_timed_out = False
        delete_session_failure_ms: float | None = None
        driver.last_launch_start_ns = None
        try:
            driver.start()
        except Exception as error:
            delete_session_error = f"{type(error).__name__}: {error}"
            delete_session_timed_out = is_timeout_error(error)
            delete_session_failure_ms = _elapsed_ms(driver.last_launch_start_ns)
        for sample in fixture["classes"]["delete"]:
            record = make_observation("delete", sample["sample_index"], sample)
            record["sample_timeout_ms"] = arguments.sample_timeout_ms
            record["launcher_start_failure_ms"] = delete_session_failure_ms
            sample_started_ns = time.monotonic_ns()
            mutation_ns: int | None = None
            precondition_started_ns: int | None = None
            query_started_ns: int | None = None
            old_clipboard: str | None = None
            path = Path(sample["path"])
            driver.last_query_started_ns = None
            try:
                if delete_session_error:
                    message = f"delete launcher startup failed: {delete_session_error}"
                    if delete_session_timed_out:
                        raise TimeoutError(message)
                    raise RuntimeError(message)
                driver.last_query_started_ns = None
                record["precondition_query_attempted"] = True
                try:
                    precondition = driver.type_query(sample["query"], expected_path=sample["path"])
                except Exception:
                    precondition_started_ns = driver.last_query_started_ns
                    raise
                precondition_started_ns = precondition["query_started_ns"]
                record["precondition_query_ms"] = precondition["latency_ms"]
                driver.open_selected_result(sample["path"], sample["query"])
                old = driver.copy_fresh_citation_and_excerpt(
                    sample["path"], sample["expected_citation"], sample["query"]
                )
                record["precondition_citation"] = old["citation"]
                record["precondition_excerpt"] = old["excerpt"]
                if old["citation"] != sample["expected_citation"] or sample["query"] not in old["excerpt"]:
                    raise RuntimeError("pre-delete UI result was not freshly authorized with its original cited passage")
                old_clipboard = old["citation"]
                mutation_ns = time.monotonic_ns()
                record["source_change"] = {
                    "operation": "unlink",
                    "path": str(path),
                    "previous_sha256": sample["initial_sha256"],
                    "unlink_completed": False,
                    "directory_fsynced": False,
                }
                path.unlink()
                record["source_change"]["unlink_completed"] = True
                directory_fd = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
                try:
                    os.fsync(directory_fd)
                    record["source_change"]["directory_fsynced"] = True
                finally:
                    os.close(directory_fd)
                stale = driver.verify_stale_evidence_denied(old_clipboard)
                record["stale_evidence_denied"] = stale["clipboard_unchanged"]
                record["stale_reauthorization"] = {
                    "message": stale["denial_message"],
                    "clipboard_unchanged": stale["clipboard_unchanged"],
                    "mutation_to_denial_ms": stale["mutation_to_denial_ms"],
                    "authorized_daemon_status_after_denial": stale["authorized_daemon_status_after_denial"],
                }
                record["source_change_to_reauthorization_ms"] = (
                    time.monotonic_ns() - mutation_ns
                ) / 1_000_000.0
                record["atspi_error_text"] = stale["denial_message"]
                driver.return_to_results()
                driver.last_query_started_ns = None
                record["fresh_query_attempted"] = True
                result = driver.type_query(sample["query"], expect_no_match=True)
                query_started_ns = result["query_started_ns"]
                record["query_to_result_ms"] = result["latency_ms"]
                record["source_change_to_fresh_result_ms"] = (
                    result["observed_ns"] - mutation_ns
                ) / 1_000_000.0
                record["post_change_old_query_absent"] = result["kind"] == "no_match"
                record["atspi_result_label"] = result["result_label"]
                record["query_latency_le_100ms"] = result["latency_ms"] <= INTERNAL_DEADLINE_MS
                record["result_observed"] = True
                record["outcome"] = (
                    "passed"
                    if record["stale_evidence_denied"] and record["post_change_old_query_absent"]
                    else "failed"
                )
                driver.return_to_results()
            except Exception as error:
                record["outcome"] = "timeout" if is_timeout_error(error) else "failed"
                record["error"] = f"{type(error).__name__}: {error}"
                record["atspi_error_text"] = str(error)
                if record["fresh_query_attempted"] and query_started_ns is None:
                    query_started_ns = driver.last_query_started_ns
            finally:
                if (
                    precondition_started_ns is None
                    and record["precondition_query_attempted"]
                    and not record["fresh_query_attempted"]
                ):
                    precondition_started_ns = driver.last_query_started_ns
                if record["precondition_query_ms"] is None and precondition_started_ns is not None:
                    record["precondition_query_ms"] = _elapsed_ms(precondition_started_ns)
                if query_started_ns is None and record["fresh_query_attempted"]:
                    query_started_ns = driver.last_query_started_ns
                if record["fresh_query_attempted"] and query_started_ns is not None:
                    record["query_input_attempted"] = True
                    if record["query_to_result_ms"] is None:
                        record["query_to_result_ms"] = _elapsed_ms(query_started_ns)
                        record["query_latency_le_100ms"] = (
                            record["query_to_result_ms"] <= INTERNAL_DEADLINE_MS
                        )
                record["query_input_attempted"] = (
                    record["precondition_query_attempted"] or record["fresh_query_attempted"]
                ) and (precondition_started_ns is not None or query_started_ns is not None)
                if record["source_change"] is not None:
                    try:
                        record["source_change"]["actual_file_exists"] = path.exists() or path.is_symlink()
                    except Exception as inspection_error:
                        record["source_change"]["final_state_error"] = (
                            f"{type(inspection_error).__name__}: {inspection_error}"
                        )
                source_change = record["source_change"]
                record["source_change_durable_verified"] = bool(
                    source_change is not None
                    and source_change.get("unlink_completed") is True
                    and source_change.get("directory_fsynced") is True
                    and source_change.get("actual_file_exists") is False
                )
                if record["outcome"] == "passed" and not record["source_change_durable_verified"]:
                    record["outcome"] = "failed"
                    record["error"] = "deleted fixture path or directory durability did not match the expected source change"
                record["latency_ms_including_failures"] = (
                    record["source_change_to_fresh_result_ms"]
                    if record["source_change_to_fresh_result_ms"] is not None
                    else (
                        delete_session_failure_ms
                        if delete_session_error is not None
                        else _elapsed_ms(mutation_ns or sample_started_ns)
                    )
                )
                record["latency_observation_scope"] = (
                    "durable source deletion to explicit AT-SPI no-match result"
                    if mutation_ns is not None
                    else (
                        "delete launcher startup attempt to its recorded failure"
                        if delete_session_error is not None
                        else "sample start through a failure before the durable source deletion"
                    )
                )
                record["latency_censored_at_timeout"] = (
                    record["outcome"] == "timeout" and not record["result_observed"]
                )
                record_observation(observations_path, observations, record)
                class_records["delete"].append(record)
                if driver.launcher is not None and driver.launcher.poll() is None:
                    try:
                        driver.return_to_results()
                    except Exception:
                        pass
        driver.close()
    finally:
        driver.close()
        driver.log_stream.close()

    hardware_report = json.loads((output_dir / "hardware.json").read_text(encoding="utf-8"))
    reference_gate = hardware_report.get("reference_hardware_gate", {})
    class_summaries: dict[str, Any] = {}
    failure_count = 0
    total_under_100 = 0
    total_latency_count = 0
    for kind in ("cold", "active", "edit", "delete"):
        rows = class_records[kind]
        latencies = [
            float(row["latency_ms_including_failures"])
            for row in rows
            if row.get("latency_ms_including_failures") is not None
        ]
        query_latencies = [
            float(row["query_to_result_ms"])
            for row in rows
            if row.get("query_to_result_ms") is not None
        ]
        outcomes: dict[str, int] = defaultdict(int)
        for row in rows:
            outcomes[str(row["outcome"])] += 1
            if row["outcome"] != "passed":
                failure_count += 1
            if row.get("query_latency_le_100ms") is True:
                total_under_100 += 1
            if row.get("query_latency_le_100ms") is not None:
                total_latency_count += 1
        query_summary = percentile_summary(query_latencies)
        query_p95 = query_summary["p95_ms"]
        class_summary: dict[str, Any] = {
            "observation_count": len(rows),
            "expected_observation_count": SAMPLES_PER_CLASS,
            "outcomes": dict(sorted(outcomes.items())),
            "latency_ms_including_failures": percentile_summary(latencies),
            "failure_inclusive_duration_observation_count": len(latencies),
            "all_failure_inclusive_durations_present": len(latencies) == len(rows) == SAMPLES_PER_CLASS,
            "query_to_atspi_result_ms_including_failed_and_timed_out_attempts": query_summary,
            "query_latency_attempt_observation_count": len(query_latencies),
            "query_latency_under_100ms_count": sum(row.get("query_latency_le_100ms") is True for row in rows),
            "query_latency_over_100ms_count": sum(row.get("query_latency_le_100ms") is False for row in rows),
            "query_latency_100ms_p95_comparison": {
                "threshold_ms": INTERNAL_DEADLINE_MS,
                "p95_ms": query_p95,
                "result": (
                    "NOT MEASURED"
                    if query_p95 is None
                    else ("PASS" if query_p95 <= INTERNAL_DEADLINE_MS else "FAIL")
                ),
                "metric": "keyboard input to AT-SPI-observed result, including observer overhead",
                "not_a_claim_about_internal_deadline": True,
            },
            "percentiles_include_failed_and_timed_out_observations": (
                len(rows) == SAMPLES_PER_CLASS and len(latencies) == SAMPLES_PER_CLASS
            ),
        }
        if kind == "cold":
            startup_latencies = [
                float(row["launcher_start_to_result_or_failure_ms"])
                for row in rows
                if row.get("launcher_start_to_result_or_failure_ms") is not None
            ]
            class_summary["launcher_start_to_result_or_failure_ms_including_failures"] = (
                percentile_summary(startup_latencies)
            )
        if kind in {"edit", "delete"}:
            changed_result_latencies = [
                float(row["source_change_to_fresh_result_ms"])
                for row in rows
                if row.get("source_change_to_fresh_result_ms") is not None
            ]
            reauthorization_latencies = [
                float(row["source_change_to_reauthorization_ms"])
                for row in rows
                if row.get("source_change_to_reauthorization_ms") is not None
            ]
            class_summary["source_change_to_fresh_result_ms_observed"] = percentile_summary(
                changed_result_latencies
            )
            class_summary["source_change_to_fresh_result_observation_count"] = len(
                changed_result_latencies
            )
            class_summary["source_change_to_reauthorization_ms_observed"] = percentile_summary(
                reauthorization_latencies
            )
            precondition_latencies = [
                float(row["precondition_query_ms"])
                for row in rows
                if row.get("precondition_query_ms") is not None
            ]
            class_summary["precondition_query_to_atspi_result_ms_including_failed_attempts"] = (
                percentile_summary(precondition_latencies)
            )
            class_summary["precondition_query_observation_count"] = len(
                precondition_latencies
            )
            if kind == "edit":
                absence_latencies = [
                    float(row["old_query_absence_ms"])
                    for row in rows
                    if row.get("old_query_absence_ms") is not None
                ]
                class_summary["old_query_absence_ms_including_failed_attempts"] = (
                    percentile_summary(absence_latencies)
                )
                class_summary["old_query_absence_observation_count"] = len(
                    absence_latencies
                )
        class_summaries[kind] = class_summary

    observed_counts = {kind: len(class_records[kind]) for kind in ("cold", "active", "edit", "delete")}
    all_counts_complete = all(count == SAMPLES_PER_CLASS for count in observed_counts.values())
    all_failure_durations_present = all(
        summary["all_failure_inclusive_durations_present"] for summary in class_summaries.values()
    )
    credential_is_private = Path(arguments.credential_file).stat().st_mode & 0o777 == 0o600
    workload_pass = (
        all_counts_complete
        and all_failure_durations_present
        and failure_count == 0
        and desktop_validation["passed"]
        and daemon_report.get("passed") is True
        and credential_is_private
    )
    reference_hardware_pass = reference_gate.get("passed") is True
    all_pass = workload_pass and reference_hardware_pass
    acceptance_failures: list[str] = []
    if not all_counts_complete:
        acceptance_failures.append(f"observation counts incomplete: {observed_counts}")
    if not all_failure_durations_present:
        acceptance_failures.append("one or more observations lack a failure-inclusive duration")
    if failure_count:
        acceptance_failures.append(f"{failure_count} interaction observations failed or timed out")
    if not desktop_validation["passed"]:
        acceptance_failures.append("private 500-entry desktop catalog validation failed")
    if daemon_report.get("passed") is not True:
        acceptance_failures.append(f"installed private search daemon provenance failed: {daemon_report.get('error')}")
    if not credential_is_private:
        acceptance_failures.append("external search grant credential is not mode 0600")
    if not reference_hardware_pass:
        acceptance_failures.extend(reference_gate.get("failures", ["reference hardware was not established"]))
    summary = {
        "schema_version": 1,
        "result": "PASS" if all_pass else "FAIL",
        "workload_result": "PASS" if workload_pass else "FAIL",
        "installed_artifact_ui_acceptance": "measured on exact dpkg-installed Debian binaries; never substituted a source-built launcher or daemon-only performance probe",
        "benchmark_elapsed_seconds": (time.monotonic_ns() - benchmark_start_ns) / 1_000_000_000,
        "sample_timeout_ms": arguments.sample_timeout_ms,
        "samples_per_class_required": SAMPLES_PER_CLASS,
        "observations_per_class": observed_counts,
        "total_observations": len(observations),
        "class_summaries": class_summaries,
        "all_fixed_observation_counts_complete": all_counts_complete,
        "all_observations_have_failure_inclusive_durations": all_failure_durations_present,
        "all_observations_passed": failure_count == 0,
        "failed_or_timed_out_observations": failure_count,
        "acceptance_failures": acceptance_failures,
        "desktop_catalog_validation": desktop_validation,
        "native_ui_evidence": ui_evidence,
        "search_daemon_process_provenance": daemon_report,
        "internal_interactive_deadline_ms": INTERNAL_DEADLINE_MS,
        "internal_deadline_directly_observable": False,
        "internal_deadline_status": "NOT DIRECTLY MEASURABLE from the installed black-box UI; no pass/fail claim is made about the internal deadline",
        "ui_end_to_end_100ms_comparison": {
            "comparison_metric": "keyboard input to AT-SPI-observed result; includes application work, accessibility polling, and observer overhead",
            "count_at_or_below_100ms": total_under_100,
            "count_over_100ms": total_latency_count - total_under_100,
            "per_class_p95_comparison": {
                kind: class_summaries[kind]["query_latency_100ms_p95_comparison"]
                for kind in ("cold", "active", "edit", "delete")
            },
            "status": "reported separately and not substituted for the internal 100 ms deadline",
        },
        "percentile_method": "nearest-rank p50/p95/p99 and maximum; every primary per-class duration includes failures and timeouts, primary query-to-result distributions include every initiated primary query, edit/delete precondition queries and edit stale-query-absence queries are reported separately, and each row records measurement scope and whether the value is right-censored at an operation timeout",
        "reference_hardware_gate": reference_gate,
        "resource_telemetry_gate": {
            "status": "PENDING private-session shutdown validation",
            "summary_file": str(output_dir / "resource-summary.json"),
        },
        "authorization_and_source_mutation": {
            "single_exact_approved_read_root": fixture["approved_root"],
            "credential_file_is_private": credential_is_private,
            "edit_checks": "old evidence ID is re-opened after mutation and must be denied; new query must show exact fresh citation/excerpt; old query must have no fresh result",
            "delete_checks": "old evidence ID is re-opened after unlink and must be denied without changing the private clipboard; fresh query must expose an explicit no-matching-files/cited-passages AT-SPI status",
        },
        "resource_telemetry_file": str(output_dir / "resource-samples.jsonl"),
        "resource_summary_file": str(output_dir / "resource-summary.json"),
        "hardware_provenance_file": str(output_dir / "hardware.json"),
        "package_provenance_file": str(output_dir / "provenance.json"),
        "fixture_manifest_file": str(output_dir / "fixture-manifest.json"),
        "indexing_preflight_file": str(output_dir / "indexing-wait.json"),
        "daemon_process_file": str(output_dir / "daemon-process.json"),
    }
    json_write(output_dir / "native-ui-evidence.json", ui_evidence)
    json_write(output_dir / "desktop-catalog-validation.json", desktop_validation)
    json_write(output_dir / "summary.json", summary)
    if not all_counts_complete:
        fail(f"did not write 200 observations per class: {observed_counts}")
    if not all_pass:
        fail(
            "installed native benchmark acceptance failed: "
            + "; ".join(acceptance_failures)
            + f"; full observations are in {observations_path}"
        )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    prepare_parser = subparsers.add_parser("prepare")
    prepare_parser.add_argument("--approved-root", required=True)
    prepare_parser.add_argument("--data-home", required=True)
    prepare_parser.add_argument("--output-dir", required=True)
    prepare_parser.set_defaults(function=prepare)

    provenance_parser = subparsers.add_parser("provenance")
    provenance_parser.add_argument("--launcher-package", required=True)
    provenance_parser.add_argument("--launcher-deb", required=True)
    provenance_parser.add_argument("--launcher-binary", required=True)
    provenance_parser.add_argument("--search-package", required=True)
    provenance_parser.add_argument("--search-deb", required=True)
    provenance_parser.add_argument("--search-binary", required=True)
    provenance_parser.add_argument("--approved-root", required=True)
    provenance_parser.add_argument("--output-dir", required=True)
    provenance_parser.set_defaults(function=provenance)

    record_parser = subparsers.add_parser("record-process")
    record_parser.add_argument("--resource-targets-file", required=True)
    record_parser.add_argument("--pid", type=int, required=True)
    record_parser.add_argument("--expected-binary", required=True)
    record_parser.set_defaults(function=record_process)

    wait_parser = subparsers.add_parser("wait-index")
    wait_parser.add_argument("--search-binary", required=True)
    wait_parser.add_argument("--socket-path", required=True)
    wait_parser.add_argument("--consumer-realm", required=True)
    wait_parser.add_argument("--credential-file", required=True)
    wait_parser.add_argument("--expected-files", type=int, required=True)
    wait_parser.add_argument("--timeout-seconds", type=float, required=True)
    wait_parser.add_argument("--output-dir", required=True)
    wait_parser.set_defaults(function=wait_index)

    monitor_parser = subparsers.add_parser("monitor")
    monitor_parser.add_argument("--output-dir", required=True)
    monitor_parser.add_argument("--resource-targets-file", required=True)
    monitor_parser.add_argument("--interval-ms", type=int, default=100)
    monitor_parser.set_defaults(function=run_monitor)

    drive_parser = subparsers.add_parser("drive")
    drive_parser.add_argument("--manifest", required=True)
    drive_parser.add_argument("--output-dir", required=True)
    drive_parser.add_argument("--launcher-binary", required=True)
    drive_parser.add_argument("--search-binary", required=True)
    drive_parser.add_argument("--socket-path", required=True)
    drive_parser.add_argument("--consumer-realm", required=True)
    drive_parser.add_argument("--credential-file", required=True)
    drive_parser.add_argument("--daemon-pid", type=int, required=True)
    drive_parser.add_argument("--process-state", required=True)
    drive_parser.add_argument("--resource-targets-file", required=True)
    drive_parser.add_argument("--sample-timeout-ms", type=int, default=DEFAULT_SAMPLE_TIMEOUT_MS)
    drive_parser.set_defaults(function=drive)
    return parser


def main() -> None:
    arguments = build_parser().parse_args()
    try:
        arguments.function(arguments)
    except subprocess.TimeoutExpired as error:
        fail(f"command timeout: {error}")
    except OSError as error:
        fail(f"operating-system prerequisite failed: {error}")


if __name__ == "__main__":
    main()

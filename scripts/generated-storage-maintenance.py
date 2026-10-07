#!/usr/bin/env python3
"""Guard generated-target capacity and prune only explicitly eligible trees.

`guard` is read-only by default. `--apply` authorizes removal of explicitly
retired target roots and Cargo incremental/fingerprint cache entries protected
by Cargo's existing nonblocking `.cargo-lock`. Target pressure is an apparent
file-size inventory only; filesystem headroom is measured independently with
statvfs and is never inferred from bytes removed.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import math
import stat
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

DEFAULT_HIGH_WATERMARK = 15.0
DEFAULT_LOW_WATERMARK = 10.0
DEFAULT_MINIMUM_FREE = 10.0
LOCK_NAMES = {".cargo-lock", "Cargo.lock", ".lock"}
SOURCE_DIRECTORY_NAMES = {".git", ".hg", "src", "source", "sources", "source-view", "crates", "examples", "scripts", "test", "tests"}
SOURCE_FILE_NAMES = {"Cargo.toml", "CMakeLists.txt", "Makefile", "meson.build"}
SOURCE_FILE_SUFFIXES = {".c", ".cc", ".cpp", ".cxx", ".h", ".hpp", ".hxx", ".java", ".js", ".jsx", ".kt", ".m", ".mm", ".py", ".rs", ".sh", ".swift", ".ts", ".tsx"}
PRESERVED_RECORD_DIRECTORIES = {"canonical", "captures", "evidence", "first-outcome", "first_outcome", "records", "results"}
PRESERVED_RECORD_LIMIT = 1024 * 1024
SAFE_CARGO_CACHE_NAMES = ("incremental", ".fingerprint")
CARGO_PROFILE_MARKERS = {".fingerprint", "build", "deps", "incremental"}


def _cargo_profile_directory(descriptor: int) -> bool:
    lock = _stat_at(descriptor, ".cargo-lock")
    if lock is not None and stat.S_ISREG(lock.st_mode) and not stat.S_ISLNK(lock.st_mode):
        return True
    markers = 0
    for name in CARGO_PROFILE_MARKERS:
        info = _stat_at(descriptor, name)
        if info is not None and stat.S_ISDIR(info.st_mode) and not stat.S_ISLNK(info.st_mode):
            markers += 1
    return markers >= 2
 
 
def _source_name(name: str) -> bool:
    return name in SOURCE_DIRECTORY_NAMES or name in SOURCE_FILE_NAMES or Path(name).suffix.lower() in SOURCE_FILE_SUFFIXES
 
 
def _canonical_record(name: str, info: os.stat_result) -> bool:
    if not stat.S_ISREG(info.st_mode) or info.st_size > PRESERVED_RECORD_LIMIT:
        return False
    lower = name.lower()
    return Path(lower).suffix == ".json" or any(
        token in lower
        for token in ("accepted", "attestation", "authorization", "final", "first-outcome", "first_outcome", "manifest", "outcome")
    )


class MaintenanceError(Exception):
    """A fail-closed path or filesystem condition."""


class UnsafeTree(MaintenanceError):
    """A symlink or identity boundary made an operation unsafe."""


class MountBoundaryError(UnsafeTree):
    """A directory entry belongs to another mount, including a bind mount."""


@dataclass(frozen=True)
class Candidate:
    parent: tuple[str, ...]
    name: str
    kind: str
    apparent_bytes: int
    identity: tuple[int, int, int]
    parent_identity: tuple[tuple[int, int, int, int], ...]
    is_directory: bool

    @property
    def relative(self) -> tuple[str, ...]:
        return (*self.parent, self.name)


@dataclass
class CargoLock:
    parent: tuple[str, ...]
    name: str
    descriptor: int
    identity: tuple[int, int, int]
    parent_identity: tuple[int, int, int, int]
    mount_id: int


@dataclass(frozen=True)
class Inventory:
    apparent_bytes: int
    top_level_bytes: dict[str, int]
    top_level_names: tuple[str, ...]
    mount_count: int
    unclassified_count: int

    @property
    def complete(self) -> bool:
        return self.mount_count == 0 and self.unclassified_count == 0


class RootAnchor:
    """Hold and revalidate the target path, then access descendants by dirfd."""

    def __init__(self, path: Path) -> None:
        self.path = Path(os.path.abspath(os.fspath(path)))
        self.path_descriptors: list[int] = []
        self.path_identities: list[tuple[int, int, int, int]] = []
        if self.path.name != "target":
            raise UnsafeTree("target root must be a directory named 'target'")
        try:
            descriptor = os.open("/", os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            self.path_descriptors.append(descriptor)
            self.path_identities.append((*_identity(os.fstat(descriptor)), _fd_mount_id(descriptor)))
            for component in self.path.parts[1:]:
                before = os.stat(component, dir_fd=descriptor, follow_symlinks=False)
                if not stat.S_ISDIR(before.st_mode) or stat.S_ISLNK(before.st_mode):
                    raise UnsafeTree("target path contains a symlink or non-directory component")
                next_descriptor = os.open(
                    component,
                    os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                    dir_fd=descriptor,
                )
                after = os.fstat(next_descriptor)
                if _identity(after) != _identity(before):
                    os.close(next_descriptor)
                    raise UnsafeTree("target path changed while it was opened")
                descriptor = next_descriptor
                self.path_descriptors.append(descriptor)
                self.path_identities.append((*_identity(after), _fd_mount_id(descriptor)))
            self.descriptor = self.path_descriptors[-1]
            self.target_identity = self.path_identities[-1]
            self.mount_id = self.target_identity[3]
            self.device = self.target_identity[0]
        except BaseException:
            self.close()
            raise

    def close(self) -> None:
        while self.path_descriptors:
            descriptor = self.path_descriptors.pop()
            try:
                os.close(descriptor)
            except OSError:
                pass

    def validate_target_path(self) -> None:
        descriptor = os.open("/", os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
        try:
            identity = (*_identity(os.fstat(descriptor)), _fd_mount_id(descriptor))
            if identity != self.path_identities[0]:
                raise UnsafeTree("filesystem root identity changed")
            for index, component in enumerate(self.path.parts[1:], start=1):
                before = os.stat(component, dir_fd=descriptor, follow_symlinks=False)
                if not stat.S_ISDIR(before.st_mode) or stat.S_ISLNK(before.st_mode):
                    raise UnsafeTree("target ancestor was replaced by a non-directory or symlink")
                next_descriptor = None
                try:
                    next_descriptor = os.open(
                        component,
                        os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                        dir_fd=descriptor,
                    )
                    actual = (*_identity(os.fstat(next_descriptor)), _fd_mount_id(next_descriptor))
                    if actual != self.path_identities[index]:
                        raise UnsafeTree("target path identity changed during maintenance")
                except BaseException:
                    if next_descriptor is not None:
                        os.close(next_descriptor)
                    raise
                os.close(descriptor)
                descriptor = next_descriptor
        finally:
            os.close(descriptor)

    def open_directory(self, relative: tuple[str, ...]) -> tuple[list[int], tuple[tuple[int, int, int, int], ...]]:
        descriptors = [os.dup(self.descriptor)]
        identities = [self.target_identity]
        try:
            for component in relative:
                parent = descriptors[-1]
                before = os.stat(component, dir_fd=parent, follow_symlinks=False)
                if not stat.S_ISDIR(before.st_mode) or stat.S_ISLNK(before.st_mode):
                    raise UnsafeTree("path contains a symlink or non-directory component")
                child = os.open(
                    component,
                    os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                    dir_fd=parent,
                )
                try:
                    identity = (*_identity(os.fstat(child)), _fd_mount_id(child))
                    if identity[:3] != _identity(before):
                        raise UnsafeTree("directory identity changed while opening a target descendant")
                    if identity[3] != self.mount_id:
                        raise MountBoundaryError("target descendant crosses a mount boundary")
                except BaseException:
                    os.close(child)
                    raise
                descriptors.append(child)
                identities.append(identity)
            return descriptors, tuple(identities)
        except BaseException:
            _close_descriptors(descriptors)
            raise

    def relative_path(self, raw: str, label: str, *, allow_equal: bool, require_existing: bool) -> tuple[str, ...]:
        path = Path(os.path.abspath(raw))
        try:
            relative = path.relative_to(self.path)
        except ValueError as error:
            raise UnsafeTree(f"{label} must be inside the target root") from error
        components = relative.parts
        if not components and not allow_equal:
            raise UnsafeTree(f"{label} cannot be the target root")
        if require_existing:
            descriptors, _ = self.open_directory(components)
            _close_descriptors(descriptors)
        else:
            current = os.dup(self.descriptor)
            try:
                for component in components:
                    try:
                        info = os.stat(component, dir_fd=current, follow_symlinks=False)
                    except FileNotFoundError:
                        break
                    if stat.S_ISLNK(info.st_mode):
                        raise UnsafeTree(f"{label} contains a symlink component")
                    if not stat.S_ISDIR(info.st_mode):
                        raise UnsafeTree(f"{label} contains a non-directory component")
                    child = os.open(
                        component,
                        os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                        dir_fd=current,
                    )
                    identity = (*_identity(os.fstat(child)), _fd_mount_id(child))
                    if identity[:3] != _identity(info) or identity[3] != self.mount_id:
                        os.close(child)
                        raise UnsafeTree(f"{label} identity or mount boundary changed")
                    os.close(current)
                    current = child
            finally:
                os.close(current)
        return tuple(components)


def _close_descriptors(descriptors: list[int]) -> None:
    for descriptor in reversed(descriptors):
        try:
            os.close(descriptor)
        except OSError:
            pass


def _identity(info: os.stat_result) -> tuple[int, int, int]:
    return info.st_dev, info.st_ino, stat.S_IFMT(info.st_mode)


def _fd_mount_id(descriptor: int) -> int:
    """Read Linux's mount identity for an open fd, including bind mounts."""
    try:
        with open(f"/proc/self/fdinfo/{descriptor}", "r", encoding="ascii") as stream:
            for line in stream:
                if line.startswith("mnt_id:"):
                    return int(line.partition(":")[2].strip())
    except (OSError, ValueError) as error:
        raise UnsafeTree(f"cannot determine descriptor mount identity: {error}") from error
    raise UnsafeTree("descriptor mount identity is unavailable; refusing filesystem traversal")


def _stat_at(directory: int, name: str) -> os.stat_result | None:
    try:
        return os.stat(name, dir_fd=directory, follow_symlinks=False)
    except FileNotFoundError:
        return None
    except OSError as error:
        raise MaintenanceError(f"cannot inspect a target entry: {error}") from error


def _open_entry(
    anchor: RootAnchor,
    parent: int,
    name: str,
    expected: os.stat_result,
    *,
    directory: bool,
) -> int:
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC
    if directory:
        flags |= os.O_DIRECTORY
    elif hasattr(os, "O_PATH"):
        flags = os.O_PATH | os.O_NOFOLLOW | os.O_CLOEXEC
    descriptor = os.open(name, flags, dir_fd=parent)
    try:
        actual_info = os.fstat(descriptor)
        if _identity(actual_info) != _identity(expected):
            raise UnsafeTree("target entry identity changed while opening it")
        if _fd_mount_id(descriptor) != anchor.mount_id:
            raise MountBoundaryError("target entry crosses a mount boundary, including a bind mount")
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def _is_prefix(left: tuple[str, ...], right: tuple[str, ...]) -> bool:
    return len(left) <= len(right) and right[: len(left)] == left


def _overlap(left: tuple[str, ...], right: tuple[str, ...]) -> bool:
    return _is_prefix(left, right) or _is_prefix(right, left)


def _protected(candidate: tuple[str, ...], roots: list[tuple[str, ...]]) -> bool:
    return any(_overlap(candidate, root) for root in roots)


def _protected_ancestor(candidate: tuple[str, ...], roots: list[tuple[str, ...]]) -> bool:
    """A protected ancestor excludes a subtree; protected descendants do not."""
    return any(_is_prefix(root, candidate) for root in roots)


def _tree_apparent(anchor: RootAnchor, relative: tuple[str, ...]) -> int:
    descriptors, _ = anchor.open_directory(relative)
    total = 0
    pending: list[int] = []
    try:
        pending.append(os.dup(descriptors[-1]))
        while pending:
            directory = pending.pop()
            try:
                for name in os.listdir(directory):
                    info = _stat_at(directory, name)
                    if info is None:
                        continue
                    if stat.S_ISDIR(info.st_mode):
                        pending.append(_open_entry(anchor, directory, name, info, directory=True))
                    elif stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode):
                        total += info.st_size
            finally:
                os.close(directory)
        return total
    finally:
        _close_descriptors(pending)
        _close_descriptors(descriptors)


def _inventory(anchor: RootAnchor) -> Inventory:
    total = 0
    top_level_bytes: dict[str, int] = {}
    unclassified = 0
    mounts = 0
    pending: list[tuple[int, str]] = []
    target = os.dup(anchor.descriptor)
    try:
        names = tuple(sorted(os.listdir(target)))
        for name in names:
            info = _stat_at(target, name)
            if info is None:
                continue
            top_level_bytes.setdefault(name, 0)
            if stat.S_ISDIR(info.st_mode):
                try:
                    child = _open_entry(anchor, target, name, info, directory=True)
                except MountBoundaryError:
                    mounts += 1
                else:
                    pending.append((child, name))
            elif stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode):
                total += info.st_size
                top_level_bytes[name] += info.st_size
            else:
                unclassified += 1
        while pending:
            directory, root_name = pending.pop()
            try:
                for name in os.listdir(directory):
                    info = _stat_at(directory, name)
                    if info is None:
                        continue
                    if stat.S_ISDIR(info.st_mode):
                        try:
                            child = _open_entry(anchor, directory, name, info, directory=True)
                        except MountBoundaryError:
                            mounts += 1
                        else:
                            pending.append((child, root_name))
                    elif stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode):
                        total += info.st_size
                        top_level_bytes[root_name] += info.st_size
                    else:
                        unclassified += 1
            finally:
                os.close(directory)
        return Inventory(total, top_level_bytes, names, mounts, unclassified)
    finally:
        for descriptor, _ in pending:
            try:
                os.close(descriptor)
            except OSError:
                pass
        os.close(target)


def _source_marker_present(
    anchor: RootAnchor,
    relative: tuple[str, ...],
    *,
    generated_cache: bool = False,
) -> bool:
    if not generated_cache and any(_source_name(part) for part in relative):
        return True
    if any(part in PRESERVED_RECORD_DIRECTORIES for part in relative):
        return False
    descriptors, _ = anchor.open_directory(relative)
    pending: list[tuple[int, bool, bool]] = []
    found = False
    try:
        root = os.dup(descriptors[-1])
        pending.append((root, _cargo_profile_directory(root), generated_cache))
        while pending and not found:
            directory, cargo_profile, generated = pending.pop()
            try:
                for name in os.listdir(directory):
                    info = _stat_at(directory, name)
                    if not generated and _source_name(name):
                        found = True
                        break
                    if name in PRESERVED_RECORD_DIRECTORIES:
                        continue
                    if cargo_profile and name in CARGO_PROFILE_MARKERS:
                        continue
                    if info is not None and stat.S_ISDIR(info.st_mode):
                        try:
                            child = _open_entry(anchor, directory, name, info, directory=True)
                        except MountBoundaryError:
                            found = True
                            break
                        child_profile = _cargo_profile_directory(child)
                        child_generated = generated or (
                            cargo_profile and name in CARGO_PROFILE_MARKERS
                        )
                        pending.append((child, child_profile, child_generated))
            finally:
                os.close(directory)
        return found
    finally:
        for descriptor, _, _ in pending:
            try:
                os.close(descriptor)
            except OSError:
                pass
        _close_descriptors(descriptors)


def _cargo_lock_exists(anchor: RootAnchor, parent: tuple[str, ...]) -> bool:
    descriptors, _ = anchor.open_directory(parent)
    try:
        return _stat_at(descriptors[-1], ".cargo-lock") is not None
    finally:
        _close_descriptors(descriptors)


def _acquire_cargo_lock(
    anchor: RootAnchor,
    parent: tuple[str, ...],
    locks_by_path: dict[tuple[str, ...], CargoLock | None],
    acquired: list[CargoLock],
) -> tuple[bool, str | None]:
    lock_relative = (*parent, ".cargo-lock")
    if lock_relative in locks_by_path:
        lock = locks_by_path[lock_relative]
        return lock is not None, None if lock is not None else "Cargo output lock is held or unsafe"
    descriptors, parent_identities = anchor.open_directory(parent)
    directory = descriptors[-1]
    try:
        info = _stat_at(directory, ".cargo-lock")
        if info is None:
            locks_by_path[lock_relative] = None
            return False, "Cargo output lock is absent"
        if not stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode):
            locks_by_path[lock_relative] = None
            return False, "Cargo output lock is not a regular file"
        lock_fd: int | None = None
        try:
            lock_fd = os.open(".cargo-lock", os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=directory)
            opened = os.fstat(lock_fd)
            identity = _identity(opened)
            if identity != _identity(info) or _fd_mount_id(lock_fd) != anchor.mount_id:
                locks_by_path[lock_relative] = None
                return False, "Cargo output lock identity or mount changed"
            try:
                fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                locks_by_path[lock_relative] = None
                return False, "Cargo output lock is held"
            except OSError as error:
                locks_by_path[lock_relative] = None
                return False, f"Cargo output lock cannot be acquired: {error}"
            lock = CargoLock(parent, ".cargo-lock", lock_fd, identity, parent_identities[-1], anchor.mount_id)
            acquired.append(lock)
            locks_by_path[lock_relative] = lock
            lock_fd = None
            return True, None
        finally:
            if lock_fd is not None:
                os.close(lock_fd)
    finally:
        _close_descriptors(descriptors)


def _cargo_locks_in_tree(
    anchor: RootAnchor,
    relative: tuple[str, ...],
    locks_by_path: dict[tuple[str, ...], CargoLock | None],
    acquired: list[CargoLock],
) -> tuple[bool, str | None]:
    descriptors, _ = anchor.open_directory(relative)
    pending: list[tuple[int, tuple[str, ...]]] = []
    try:
        pending.append((os.dup(descriptors[-1]), relative))
        while pending:
            directory, directory_path = pending.pop()
            try:
                for name in os.listdir(directory):
                    info = _stat_at(directory, name)
                    if info is None:
                        continue
                    if name == ".cargo-lock":
                        okay, reason = _acquire_cargo_lock(
                            anchor, directory_path, locks_by_path, acquired
                        )
                        if not okay:
                            return False, reason or "Cargo output lock unavailable"
                    elif stat.S_ISDIR(info.st_mode):
                        pending.append(
                            (_open_entry(anchor, directory, name, info, directory=True), (*directory_path, name))
                        )
            finally:
                os.close(directory)
        return True, None
    finally:
        for descriptor, _ in pending:
            try:
                os.close(descriptor)
            except OSError:
                pass
        _close_descriptors(descriptors)


def _candidate(
    anchor: RootAnchor,
    relative: tuple[str, ...],
    kind: str,
    protected_roots: list[tuple[str, ...]],
    *,
    directory_only: bool,
) -> tuple[Candidate | None, str | None]:
    if not relative:
        raise UnsafeTree("target root itself can never be a deletion candidate")
    if _protected(relative, protected_roots):
        return None, "current, pinned, or leased"
    if kind != "compiler-cache" and any(_source_name(part) for part in relative):
        return None, "source-owned path is preserved"
    if any(part in PRESERVED_RECORD_DIRECTORIES for part in relative):
        return None, "canonical evidence root is preserved"
    parent = relative[:-1]
    descriptors, parent_identity = anchor.open_directory(parent)
    try:
        info = _stat_at(descriptors[-1], relative[-1])
        if info is None:
            return None, "root disappeared before classification"
        is_directory = stat.S_ISDIR(info.st_mode) and not stat.S_ISLNK(info.st_mode)
        if directory_only and not is_directory:
            return None, "eligible root is not a real directory"
        if not is_directory and not stat.S_ISREG(info.st_mode):
            return None, "symlink or special entry is preserved"
        entry_fd = _open_entry(anchor, descriptors[-1], relative[-1], info, directory=is_directory)
        os.close(entry_fd)
    finally:
        _close_descriptors(descriptors)
    if is_directory and _source_marker_present(
        anchor, relative, generated_cache=kind == "compiler-cache"
    ):
        return None, "source tree marker found; root is preserved"
    apparent = _tree_apparent(anchor, relative) if is_directory else info.st_size
    return Candidate(parent, relative[-1], kind, apparent, _identity(info), parent_identity, is_directory), None


def _discover_retired(
    anchor: RootAnchor,
    relative: tuple[str, ...],
    protected_roots: list[tuple[str, ...]],
    locks_by_path: dict[tuple[str, ...], CargoLock | None],
    acquired: list[CargoLock],
) -> tuple[Candidate | None, str | None]:
    if _protected(relative, protected_roots):
        return None, "current, pinned, or leased"
    if not relative:
        raise UnsafeTree("target root itself can never be retired")
    for depth in range(len(relative)):
        ancestor = relative[:depth]
        if _cargo_lock_exists(anchor, ancestor):
            okay, reason = _acquire_cargo_lock(anchor, ancestor, locks_by_path, acquired)
            if not okay:
                return None, reason
    okay, reason = _cargo_locks_in_tree(anchor, relative, locks_by_path, acquired)
    if not okay:
        return None, reason
    return _candidate(anchor, relative, "retired", protected_roots, directory_only=True)


def _discover_cargo_caches(
    anchor: RootAnchor,
    cargo_root: tuple[str, ...],
    protected_roots: list[tuple[str, ...]],
    locks_by_path: dict[tuple[str, ...], CargoLock | None],
    acquired: list[CargoLock],
    candidates: list[Candidate],
    skipped: list[dict[str, str]],
) -> None:
    if not cargo_root or _protected_ancestor(cargo_root, protected_roots):
        skipped.append({"path": "/".join(cargo_root), "reason": "Cargo output is current or pinned"})
        return
    okay, reason = _acquire_cargo_lock(anchor, cargo_root, locks_by_path, acquired)
    if not okay:
        skipped.append({"path": "/".join(cargo_root), "reason": reason or "Cargo output lock unavailable"})
        return
    root_descriptors, _ = anchor.open_directory(cargo_root)
    try:
        profile_fd = root_descriptors[-1]
        for cache_name in SAFE_CARGO_CACHE_NAMES:
            cache_info = _stat_at(profile_fd, cache_name)
            if cache_info is None:
                continue
            cache_relative = (*cargo_root, cache_name)
            if stat.S_ISLNK(cache_info.st_mode) or not stat.S_ISDIR(cache_info.st_mode):
                skipped.append({"path": "/".join(cache_relative), "reason": "unclassified cache root is preserved"})
                continue
            try:
                cache_fd = _open_entry(anchor, profile_fd, cache_name, cache_info, directory=True)
            except MountBoundaryError as error:
                skipped.append({"path": "/".join(cache_relative), "reason": str(error)})
                continue
            try:
                for name in sorted(os.listdir(cache_fd)):
                    relative = (*cache_relative, name)
                    if name in LOCK_NAMES or name.endswith(".lock"):
                        skipped.append({"path": "/".join(relative), "reason": "lock identity is preserved"})
                        continue
                    info = _stat_at(cache_fd, name)
                    if info is None:
                        continue
                    if stat.S_ISLNK(info.st_mode) or not (
                        stat.S_ISDIR(info.st_mode)
                        or (cache_name == ".fingerprint" and stat.S_ISREG(info.st_mode))
                    ):
                        skipped.append({"path": "/".join(relative), "reason": "unclassified Cargo output is preserved"})
                        continue
                    cache_candidate, why = _candidate(
                        anchor,
                        relative,
                        "compiler-cache",
                        protected_roots,
                        directory_only=stat.S_ISDIR(info.st_mode),
                    )
                    if cache_candidate is None:
                        skipped.append({"path": "/".join(relative), "reason": why or "not eligible"})
                    else:
                        candidates.append(cache_candidate)
            finally:
                os.close(cache_fd)
        for name in ("deps", "build"):
            if _stat_at(profile_fd, name) is not None:
                skipped.append({"path": "/".join((*cargo_root, name)), "reason": "Cargo deps/build outputs may contain executables or runtime inputs"})
    finally:
        _close_descriptors(root_descriptors)


def _verify_locks(anchor: RootAnchor, locks: list[CargoLock]) -> None:
    anchor.validate_target_path()
    for lock in locks:
        descriptors, parent_identities = anchor.open_directory(lock.parent)
        try:
            if parent_identities[-1] != lock.parent_identity:
                raise UnsafeTree("Cargo output lock parent identity changed during maintenance")
            held_info = os.fstat(lock.descriptor)
            if _identity(held_info) != lock.identity or _fd_mount_id(lock.descriptor) != lock.mount_id:
                raise UnsafeTree("held Cargo output lock identity or mount changed during maintenance")
            info = _stat_at(descriptors[-1], lock.name)
            if info is None or _identity(info) != lock.identity:
                raise UnsafeTree("Cargo output lock path identity changed during maintenance")
            descriptor = os.open(
                lock.name,
                os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC,
                dir_fd=descriptors[-1],
            )
            try:
                if _identity(os.fstat(descriptor)) != lock.identity or _fd_mount_id(descriptor) != lock.mount_id:
                    raise UnsafeTree("Cargo output lock identity or mount changed during maintenance")
            finally:
                os.close(descriptor)
        finally:
            _close_descriptors(descriptors)


def _is_lock_name(name: str) -> bool:
    return name in LOCK_NAMES or name.endswith(".lock")


def _prune_directory_contents(
    anchor: RootAnchor,
    descriptor: int,
    *,
    preserve_records: bool,
    cargo_profile: bool = False,
    cargo_generated: bool = False,
) -> tuple[bool, bool, int]:
    """Remove ordinary contents while preserving locks, links, sources, and records."""
    cargo_profile = cargo_profile or _cargo_profile_directory(descriptor)
    retained = False
    changed = False
    removed_bytes = 0
    for name in sorted(os.listdir(descriptor)):
        info = _stat_at(descriptor, name)
        if info is None:
            continue
        if (
            _is_lock_name(name)
            or (not cargo_generated and _source_name(name))
            or name in PRESERVED_RECORD_DIRECTORIES
            or (preserve_records and not cargo_generated and _canonical_record(name, info))
        ):
            retained = True
            continue
        if stat.S_ISLNK(info.st_mode):
            retained = True
            continue
        if stat.S_ISDIR(info.st_mode):
            try:
                child = _open_entry(anchor, descriptor, name, info, directory=True)
            except MountBoundaryError:
                retained = True
                continue
            try:
                child_retained, child_changed, child_removed = _prune_directory_contents(
                    anchor,
                    child,
                    preserve_records=preserve_records,
                    cargo_profile=cargo_profile or _cargo_profile_directory(child),
                    cargo_generated=cargo_generated
                    or (cargo_profile and name in CARGO_PROFILE_MARKERS),
                )
                removed_bytes += child_removed
                changed = changed or child_changed
                if child_retained:
                    retained = True
                else:
                    current = _stat_at(descriptor, name)
                    if current is None or _identity(current) != _identity(info):
                        raise UnsafeTree("candidate child identity changed before directory removal")
                    os.rmdir(name, dir_fd=descriptor)
                    changed = True
            finally:
                os.close(child)
        elif stat.S_ISREG(info.st_mode):
            file_descriptor = _open_entry(anchor, descriptor, name, info, directory=False)
            try:
                current = _stat_at(descriptor, name)
                if current is None or _identity(current) != _identity(info):
                    raise UnsafeTree("candidate file identity changed before unlink")
                os.unlink(name, dir_fd=descriptor)
                removed_bytes += info.st_size
                changed = True
            finally:
                os.close(file_descriptor)
        else:
            retained = True
    if changed:
        os.fsync(descriptor)
    return retained, changed, removed_bytes


def _prune_candidate(anchor: RootAnchor, candidate: Candidate) -> tuple[bool, bool, int, str | None]:
    anchor.validate_target_path()
    parent_descriptors, parent_identity = anchor.open_directory(candidate.parent)
    try:
        if parent_identity != candidate.parent_identity:
            raise UnsafeTree("candidate ancestor identity changed since classification")
        parent = parent_descriptors[-1]
        info = _stat_at(parent, candidate.name)
        if info is None or _identity(info) != candidate.identity:
            raise UnsafeTree("candidate identity changed since classification")
        if candidate.is_directory:
            directory = _open_entry(anchor, parent, candidate.name, info, directory=True)
            try:
                retained, changed, removed_bytes = _prune_directory_contents(
                    anchor,
                    directory,
                    preserve_records=candidate.kind == "retired",
                    cargo_generated=candidate.kind == "compiler-cache",
                )
            finally:
                os.close(directory)
            if not retained:
                current = _stat_at(parent, candidate.name)
                if current is None or _identity(current) != candidate.identity:
                    raise UnsafeTree("candidate ancestor changed before root removal")
                os.rmdir(candidate.name, dir_fd=parent)
                os.fsync(parent)
                return True, True, removed_bytes, None
            if changed:
                os.fsync(parent)
            return changed, False, removed_bytes, "Cargo locks, symlinks, source markers, or unknown entries remain"
        file_descriptor = _open_entry(anchor, parent, candidate.name, info, directory=False)
        try:
            current = _stat_at(parent, candidate.name)
            if current is None or _identity(current) != candidate.identity:
                raise UnsafeTree("candidate file identity changed before unlink")
            if _is_lock_name(candidate.name):
                return False, False, 0, "lock identity is preserved"
            os.unlink(candidate.name, dir_fd=parent)
            os.fsync(parent)
            return True, True, info.st_size, None
        finally:
            os.close(file_descriptor)
    finally:
        _close_descriptors(parent_descriptors)




def _statvfs(anchor: RootAnchor) -> tuple[int, int]:
    try:
        values = os.fstatvfs(anchor.descriptor)
    except OSError as error:
        raise MaintenanceError(f"cannot measure filesystem capacity with statvfs: {error}") from error
    total = values.f_blocks * values.f_frsize
    available = values.f_bavail * values.f_frsize
    if total <= 0:
        raise MaintenanceError("filesystem reports no measurable capacity")
    return total, available


def _percent(value: int, total: int) -> float:
    return round(100.0 * value / total, 4)


def _validate_thresholds(high: float, low: float, minimum: float) -> None:
    if not (0.0 < low < high < 100.0):
        raise MaintenanceError("watermarks must satisfy 0 < low < high < 100")
    if not (0.0 <= minimum < 100.0):
        raise MaintenanceError("minimum free percentage must satisfy 0 <= minimum < 100")


def _close_locks(locks: list[CargoLock]) -> None:
    for lock in reversed(locks):
        try:
            os.close(lock.descriptor)
        except OSError:
            pass


def _maintain_with_anchor(
    arguments: argparse.Namespace,
    anchor: RootAnchor,
    locks: list[CargoLock],
) -> tuple[dict[str, Any], int]:
    current = anchor.relative_path(
        arguments.current_root, "current root", allow_equal=True, require_existing=False
    )
    pinned = [
        anchor.relative_path(raw, "pinned root", allow_equal=True, require_existing=False)
        for raw in arguments.pinned_root
    ]
    leases = [
        anchor.relative_path(raw, "lease root", allow_equal=True, require_existing=False)
        for raw in arguments.lease_root
    ]
    protected_roots = [current, *pinned, *leases]
    retired_paths = [
        anchor.relative_path(raw, "retired root", allow_equal=False, require_existing=True)
        for raw in arguments.retired_root
    ]
    cargo_roots = [
        anchor.relative_path(raw, "Cargo target root", allow_equal=False, require_existing=True)
        for raw in arguments.cargo_target_root
    ]
    declared = [*retired_paths, *cargo_roots]
    for index, left in enumerate(declared):
        for right in declared[index + 1 :]:
            if _overlap(left, right):
                raise UnsafeTree("overlapping retired/Cargo roots are ambiguous")

    total_bytes, free_before = _statvfs(anchor)
    before_inventory = _inventory(anchor)
    minimum_free_bytes = math.ceil(total_bytes * arguments.minimum_free / 100.0)
    high_bytes = int(total_bytes * arguments.high_watermark / 100.0)
    low_bytes = int(total_bytes * arguments.low_watermark / 100.0)
    known_top = {root[0] for root in [*protected_roots, *declared] if root}
    if () in protected_roots:
        known_top.update(before_inventory.top_level_names)
    unknown_names = [name for name in before_inventory.top_level_names if name not in known_top]
    unknown_bytes = sum(before_inventory.top_level_bytes.get(name, 0) for name in unknown_names)
    target_inventory_bytes = before_inventory.apparent_bytes
    high_triggered = target_inventory_bytes > high_bytes
    minimum_free_triggered = free_before < minimum_free_bytes
    triggered = high_triggered or minimum_free_triggered
    result: dict[str, Any] = {
        "status": "ok",
        "mode": "apply" if arguments.apply else "dry-run",
        "retirement_authorization": {
            "cargo_target_root": "only incremental and fingerprint cache children are eligible; deps/build outputs remain",
            "retired_root": "explicit whole-root retirement also permits ordinary Cargo deps/build output pruning after lock checks",
        },
        "target_root": str(anchor.path),
        "filesystem_total_bytes": total_bytes,
        "free_bytes_before": free_before,
        "free_percent_before": _percent(free_before, total_bytes),
        "high_watermark_percent": arguments.high_watermark,
        "low_watermark_percent": arguments.low_watermark,
        "minimum_free_percent": arguments.minimum_free,
        "target_apparent_bytes_before": target_inventory_bytes,
        "target_apparent_bytes_after": target_inventory_bytes,
        "unknown_root_count": len(unknown_names),
        "unknown_apparent_bytes": unknown_bytes,
        "unclassified_entry_count": before_inventory.unclassified_count,
        "mount_boundary_count": before_inventory.mount_count,
        "inventory_complete": before_inventory.complete,
        "high_watermark_bytes": high_bytes,
        "low_watermark_bytes": low_bytes,
        "minimum_free_bytes": minimum_free_bytes,
        "high_watermark_triggered": high_triggered,
        "minimum_free_triggered": minimum_free_triggered,
        "maintenance_triggered": triggered,
        "eligible_roots": [],
        "preserved_or_skipped": [],
        "pruned_roots": [],
        "modified_roots": [],
    }
    if not before_inventory.complete:
        result["status"] = "measurement-blocked"
        result["capacity_ok"] = free_before >= minimum_free_bytes
        result["watermark_ok"] = False
        result["free_bytes_after"] = free_before
        result["post_sync_free_bytes"] = free_before
        result["free_percent_after"] = _percent(free_before, total_bytes)
        result["observed_free_delta_bytes"] = 0
        return result, 2

    skipped: list[dict[str, str]] = []
    candidates: list[Candidate] = []
    locks_by_path: dict[tuple[str, ...], CargoLock | None] = {}
    for path in retired_paths:
        candidate, reason = _discover_retired(anchor, path, protected_roots, locks_by_path, locks)
        if candidate is None:
            skipped.append({"path": "/".join(path), "reason": reason or "not eligible"})
        else:
            candidates.append(candidate)
    for cargo_root in cargo_roots:
        _discover_cargo_caches(
            anchor, cargo_root, protected_roots, locks_by_path, locks, candidates, skipped
        )

    result["eligible_roots"] = [
        {
            "path": "/".join(candidate.relative),
            "kind": candidate.kind,
            "apparent_bytes": candidate.apparent_bytes,
        }
        for candidate in candidates
    ]
    result["preserved_or_skipped"] = skipped
    free_after = free_before
    inventory_complete = True

    if triggered and not arguments.apply:
        result["status"] = "dry-run-maintenance-required"
    elif triggered and arguments.apply:
        ordered = sorted(
            candidates,
            key=lambda candidate: (
                candidate.kind != "retired",
                -candidate.apparent_bytes,
                candidate.relative,
            ),
        )
        for candidate in ordered:
            needs_watermark = high_triggered and target_inventory_bytes > low_bytes
            needs_capacity = free_after < minimum_free_bytes
            if not needs_watermark and not needs_capacity:
                break
            try:
                _verify_locks(anchor, locks)
                changed, fully_removed, removed_bytes, reason = _prune_candidate(anchor, candidate)
                if reason is not None:
                    skipped.append({"path": "/".join(candidate.relative), "reason": reason})
                if changed:
                    if fully_removed:
                        result["pruned_roots"].append("/".join(candidate.relative))
                    else:
                        result["modified_roots"].append("/".join(candidate.relative))
                    target_inventory_bytes = max(0, target_inventory_bytes - removed_bytes)
                    _, free_after = _statvfs(anchor)
            except (MaintenanceError, OSError) as error:
                skipped.append({"path": "/".join(candidate.relative), "reason": f"pruning stopped safely: {error}"})
                break
        anchor.validate_target_path()
        os.fsync(anchor.descriptor)
        after_inventory = _inventory(anchor)
        inventory_complete = after_inventory.complete
        target_inventory_bytes = after_inventory.apparent_bytes
        _, free_after = _statvfs(anchor)
        result["preserved_or_skipped"] = skipped

    if not arguments.apply and triggered:
        free_after = free_before
    watermark_ok = target_inventory_bytes <= low_bytes if high_triggered else True
    capacity_ok = free_after >= minimum_free_bytes
    result["target_apparent_bytes_after"] = target_inventory_bytes
    result["mount_boundary_count"] = after_inventory.mount_count if arguments.apply and triggered else before_inventory.mount_count
    result["inventory_complete"] = inventory_complete
    result["unclassified_entry_count"] = (
        after_inventory.unclassified_count
        if arguments.apply and triggered
        else before_inventory.unclassified_count
    )
    result["free_bytes_after"] = free_after
    result["post_sync_free_bytes"] = free_after
    result["free_percent_after"] = _percent(free_after, total_bytes)
    result["observed_free_delta_bytes"] = free_after - free_before
    result["capacity_ok"] = capacity_ok
    result["watermark_ok"] = watermark_ok
    if not inventory_complete:
        result["status"] = "measurement-blocked"
        return result, 2
    if not capacity_ok:
        result["status"] = "capacity-blocked"
        return result, 2
    if triggered and arguments.apply and not watermark_ok:
        result["status"] = "maintenance-blocked"
        return result, 2
    if triggered and not arguments.apply:
        return result, 2
    return result, 0


def maintain(arguments: argparse.Namespace) -> tuple[dict[str, Any], int]:
    _validate_thresholds(arguments.high_watermark, arguments.low_watermark, arguments.minimum_free)
    anchor = RootAnchor(Path(arguments.target_root))
    locks: list[CargoLock] = []
    result: dict[str, Any] | None = None
    try:
        result, exit_code = _maintain_with_anchor(arguments, anchor, locks)
        try:
            _verify_locks(anchor, locks)
        except (MaintenanceError, OSError) as error:
            result["status"] = "lock-identity-blocked"
            result["lock_identity_error"] = str(error)
            exit_code = 2
        return result, exit_code
    finally:
        _close_locks(locks)
        anchor.close()


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Guard generated target storage without guessing deletions.")
    subparsers = parser.add_subparsers(dest="command", required=True)
    guard = subparsers.add_parser("guard", help="measure target pressure and optionally prune explicit eligible roots")
    guard.add_argument("--target-root", required=True, help="existing generated directory named target")
    guard.add_argument("--current-root", required=True, help="current useful root to preserve")
    guard.add_argument("--pinned-root", action="append", default=[], help="additional current/useful root to preserve")
    guard.add_argument("--lease-root", action="append", default=[], help="live or leased input root to preserve")
    guard.add_argument("--retired-root", action="append", default=[], help="explicitly retire an owned generated root; for Cargo deps/build outputs this authorizes the whole profile/generation after lock checks")
    guard.add_argument("--cargo-target-root", action="append", default=[], help="Cargo profile output directory (e.g. target/debug); only incremental and fingerprint children are eligible")
    guard.add_argument("--high-watermark", type=float, default=DEFAULT_HIGH_WATERMARK, help="target apparent inventory trigger as percent of filesystem (default: 15)")
    guard.add_argument("--low-watermark", type=float, default=DEFAULT_LOW_WATERMARK, help="target apparent inventory target as percent of filesystem (default: 10)")
    guard.add_argument("--minimum-free", type=float, default=DEFAULT_MINIMUM_FREE, help="minimum statvfs available capacity as percent of filesystem (default: 10)")
    guard.add_argument("--apply", action="store_true", help="authorize pruning when a watermark or capacity threshold triggers; default is dry-run")
    return parser


def main(argv: list[str] | None = None) -> int:
    arguments = _parser().parse_args(argv)
    try:
        result, exit_code = maintain(arguments)
    except (MaintenanceError, OSError, ValueError) as error:
        result = {"status": "error", "error": str(error)}
        exit_code = 1
    print(json.dumps(result, sort_keys=True))
    return exit_code


if __name__ == "__main__":
    sys.exit(main())

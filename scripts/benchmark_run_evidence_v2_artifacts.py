"""Safe artifact verification and content-bound v2 evidence checks."""

from __future__ import annotations

import codecs
import errno
import hashlib
import json
import math
import mimetypes
import os
import re
import stat
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path, PurePosixPath
from typing import Any
from urllib.parse import urlsplit

_KNOWN_SUFFIX_MEDIA_TYPES = {
    ".csv": "text/csv",
    ".gz": "application/gzip",
    ".jpeg": "image/jpeg",
    ".jpg": "image/jpeg",
    ".json": "application/json",
    ".jsonl": "application/json",
    ".log": "text/plain",
    ".pdf": "application/pdf",
    ".png": "image/png",
    ".tsv": "text/tab-separated-values",
    ".txt": "text/plain",
    ".yaml": "application/yaml",
    ".yml": "application/yaml",
    ".zip": "application/zip",
}

_STRUCTURED_ROLES = {
    "protocol", "case_list", "execution", "declared_budget", "attempt_ledger",
    "access_record", "observations", "start_receipt", "terminal_receipt", "original_record",
    "system_configuration", "campaign_consumption",
}

_SOURCE_ROLE_TO_FIELD = {
    "corpus_snapshot": "snapshot_digest", "splits": "splits_digest",
    "qrels": "qrels_digest", "case_list": "case_list_digest",
}


@dataclass(frozen=True)
class VerifiedArtifact:
    artifact_id: str
    role: str
    digest: str
    content: Any | None


def _reject_constant(_value: str) -> None:
    raise ValueError("non-finite JSON number")


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON object key")
        result[key] = value
    return result


def _load_strict_json(path: Path) -> Any:
    return json.loads(
        path.read_text(encoding="utf-8"),
        parse_constant=_reject_constant,
        object_pairs_hook=_unique_object,
    )


def _matches_type(value: Any, expected: str) -> bool:
    if expected == "object":
        return isinstance(value, dict)
    if expected == "array":
        return isinstance(value, list)
    if expected == "string":
        return isinstance(value, str)
    if expected == "boolean":
        return isinstance(value, bool)
    if expected == "integer":
        return type(value) is int
    if expected == "number":
        return _finite_number(value)
    if expected == "null":
        return value is None
    return False


def _finite_number(value: Any) -> bool:
    return type(value) is int or (type(value) is float and math.isfinite(value))


def _resolve_ref(reference: str, root_schema: dict[str, Any]) -> Any:
    if not reference.startswith("#/"):
        raise ValueError("only local schema references are supported")
    target: Any = root_schema
    for component in reference[2:].split("/"):
        component = component.replace("~1", "/").replace("~0", "~")
        target = target[component]
    return target


def _json_schema_equal(left: Any, right: Any) -> bool:
    if type(left) is bool or type(right) is bool:
        return type(left) is type(right) and left == right
    if type(left) in (int, float) and type(right) in (int, float):
        return left == right
    if type(left) is not type(right):
        return False
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(
            _json_schema_equal(left[key], right[key]) for key in left
        )
    if isinstance(left, list):
        return len(left) == len(right) and all(
            _json_schema_equal(first, second) for first, second in zip(left, right)
        )
    return left == right


def _schema_errors(
    value: Any,
    schema: dict[str, Any],
    root_schema: dict[str, Any],
    location: str = "manifest",
) -> list[str]:
    if "$ref" in schema:
        try:
            target = _resolve_ref(schema["$ref"], root_schema)
        except (KeyError, TypeError, ValueError):
            return [f"{location}: schema reference is invalid"]
        return _schema_errors(value, target, root_schema, location)

    if "anyOf" in schema:
        alternatives = [
            _schema_errors(value, branch, root_schema, location)
            for branch in schema["anyOf"]
        ]
        if not any(not errors for errors in alternatives):
            return [f"{location}: value does not match an allowed schema type"]
        return []

    expected_type = schema.get("type")
    if expected_type is not None:
        types = expected_type if isinstance(expected_type, list) else [expected_type]
        if not any(_matches_type(value, item) for item in types):
            return [f"{location}: has an invalid type"]

    errors: list[str] = []
    if "const" in schema and not _json_schema_equal(value, schema["const"]):
        errors.append(f"{location}: does not match the required constant")
    if "enum" in schema and not any(
        _json_schema_equal(value, option) for option in schema["enum"]
    ):
        errors.append(f"{location}: is not an allowed value")
    if isinstance(value, str):
        if len(value) < schema.get("minLength", 0):
            errors.append(f"{location}: must not be empty")
        pattern = schema.get("pattern")
        if pattern is not None and re.search(pattern, value) is None:
            errors.append(f"{location}: has an invalid format")
    if type(value) in (int, float):
        if not _finite_number(value):
            errors.append(f"{location}: must be finite")
        if "minimum" in schema and value < schema["minimum"]:
            errors.append(f"{location}: is below the minimum")
        if "maximum" in schema and value > schema["maximum"]:
            errors.append(f"{location}: exceeds the maximum")

    if isinstance(value, dict):
        for required in schema.get("required", []):
            if required not in value:
                errors.append(f"{location}.{required}: is required")
        properties = schema.get("properties", {})
        additional = schema.get("additionalProperties", True)
        for key, item in value.items():
            if key in properties:
                child_location = f"{location}.{key}"
                errors.extend(
                    _schema_errors(item, properties[key], root_schema, child_location)
                )
            elif additional is False:
                errors.append(f"{location}: contains a field not allowed by the schema")
            elif isinstance(additional, dict):
                errors.extend(
                    _schema_errors(item, additional, root_schema, f"{location}.additionalProperty")
                )
    if isinstance(value, list):
        if len(value) < schema.get("minItems", 0):
            errors.append(f"{location}: has too few items")
        item_schema = schema.get("items")
        if isinstance(item_schema, dict):
            for index, item in enumerate(value):
                errors.extend(
                    _schema_errors(item, item_schema, root_schema, f"{location}[{index}]")
                )
    return errors


def _timestamp(value: Any) -> datetime | None:
    if not isinstance(value, str):
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        return None
    return parsed


def _safe_relative_path(value: Any) -> bool:
    if not isinstance(value, str) or not value or "\\" in value or "\x00" in value:
        return False
    try:
        value.encode("utf-8", errors="strict")
        parsed = urlsplit(value)
    except (UnicodeEncodeError, ValueError):
        return False
    if parsed.scheme or parsed.netloc or parsed.query or parsed.fragment or value.startswith("/"):
        return False
    if any(part in {"", ".", ".."} for part in value.split("/")):
        return False
    path = PurePosixPath(value)
    return not path.is_absolute() and bool(path.parts)


def _loads_strict_json(data: bytes | bytearray) -> Any:
    return json.loads(
        data.decode("utf-8", errors="strict"),
        parse_constant=_reject_constant,
        object_pairs_hook=_unique_object,
    )


def _open_directory_chain(path: Path) -> int:
    if os.open not in os.supports_dir_fd or not hasattr(os, "O_NOFOLLOW"):
        raise OSError("safe descriptor traversal is unavailable")
    directory_flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | os.O_NOFOLLOW
    current = os.open(os.path.sep, directory_flags)
    try:
        for part in Path(os.path.abspath(os.fspath(path))).parts:
            if part in {os.path.sep, "", "."}:
                continue
            next_fd = os.open(part, directory_flags, dir_fd=current)
            os.close(current)
            current = next_fd
        if not stat.S_ISDIR(os.fstat(current).st_mode):
            raise OSError("artifact root is not a directory")
        return current
    except BaseException:
        os.close(current)
        raise


def _open_relative(root_fd: int, value: str) -> int:
    flags = os.O_RDONLY | os.O_NOFOLLOW | getattr(os, "O_CLOEXEC", 0)
    directory_flags = flags | getattr(os, "O_DIRECTORY", 0)
    current = os.dup(root_fd)
    try:
        parts = PurePosixPath(value).parts
        for part in parts[:-1]:
            next_fd = os.open(part, directory_flags, dir_fd=current)
            os.close(current)
            current = next_fd
        result = os.open(parts[-1], flags | getattr(os, "O_NONBLOCK", 0), dir_fd=current)
        return result
    finally:
        os.close(current)


def _media_matches(name: str, declared: str, sample: bytes) -> bool:
    suffix = PurePosixPath(name).suffix.lower()
    expected = _KNOWN_SUFFIX_MEDIA_TYPES.get(suffix)
    if expected is None:
        expected, _ = mimetypes.guess_type(PurePosixPath(name).name, strict=False)
    if expected is not None and expected != declared:
        return False
    signatures = {
        "application/pdf": sample.startswith(b"%PDF-"),
        "application/zip": sample.startswith((b"PK\x03\x04", b"PK\x05\x06")),
        "image/jpeg": sample.startswith(b"\xff\xd8\xff"),
        "image/png": sample.startswith(b"\x89PNG\r\n\x1a\n"),
        "application/gzip": sample.startswith(b"\x1f\x8b"),
    }
    return signatures.get(declared, True)


def _same_file_snapshot(before: os.stat_result, after: os.stat_result) -> bool:
    return all(
        getattr(before, key) == getattr(after, key)
        for key in ("st_dev", "st_ino", "st_size", "st_mtime_ns", "st_ctime_ns")
    )


def _verify_artifacts(
    artifacts: list[Any], artifact_root: Path | None
) -> tuple[dict[str, VerifiedArtifact], list[str], list[str]]:
    verified: dict[str, VerifiedArtifact] = {}
    errors: list[str] = []
    warnings: list[str] = []
    if artifact_root is None:
        if artifacts:
            warnings.append("artifact bytes were not checked; all artifact references remain unverified")
        return verified, errors, warnings
    try:
        root_fd = _open_directory_chain(artifact_root)
    except (OSError, UnicodeError, RuntimeError, ValueError):
        return verified, ["artifact root is unavailable or contains a symlink"], warnings

    try:
        for index, artifact in enumerate(artifacts):
            artifact_id = artifact.get("artifact_id")
            access = artifact.get("access", {})
            classification = access.get("classification")
            if classification == "unavailable":
                warnings.append(f"artifact[{index}] is declared unavailable and remains unverified")
                continue
            if not _safe_relative_path(artifact.get("path")):
                errors.append(f"artifact[{index}].path is not a safe local path")
                continue
            try:
                fd = _open_relative(root_fd, artifact["path"])
            except OSError as error:
                if classification in {"private", "controlled"} and error.errno in {
                    errno.ENOENT, errno.EACCES, errno.EPERM
                }:
                    warnings.append(f"artifact[{index}] is inaccessible; its evidence is unverified")
                else:
                    errors.append(f"artifact[{index}] cannot be opened safely beneath the artifact root")
                continue
            except (UnicodeError, ValueError):
                errors.append(f"artifact[{index}].path is not filesystem-encodable")
                continue

            try:
                before = os.fstat(fd)
                if not stat.S_ISREG(before.st_mode):
                    errors.append(f"artifact[{index}] is not a regular file")
                    continue
                if before.st_size != artifact.get("size_bytes"):
                    errors.append(f"artifact[{index}] size does not match its manifest")
                    continue
                digest = hashlib.sha256()
                declared = artifact["media_type"]
                is_json = declared == "application/json" or declared.endswith("+json")
                text_media = declared.startswith("text/") or declared in {
                    "application/yaml", "application/x-yaml"
                }
                json_bytes = bytearray() if is_json else None
                decoder = codecs.getincrementaldecoder("utf-8")("strict") if text_media else None
                sample = bytearray()
                size = 0
                stream = os.fdopen(fd, "rb")
                fd = -1
                with stream:
                    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                        size += len(chunk)
                        digest.update(chunk)
                        if len(sample) < 8192:
                            sample.extend(chunk[:8192 - len(sample)])
                        if json_bytes is not None:
                            json_bytes.extend(chunk)
                        if decoder is not None:
                            if b"\0" in chunk:
                                raise UnicodeError("NUL in text artifact")
                            decoder.decode(chunk, final=False)
                    if decoder is not None:
                        decoder.decode(b"", final=True)
                    after = os.fstat(stream.fileno())
                if size != before.st_size or not _same_file_snapshot(before, after):
                    errors.append(f"artifact[{index}] changed while it was being verified")
                    continue
                if digest.hexdigest().lower() != artifact["sha256"].lower():
                    errors.append(f"artifact[{index}] digest does not match its manifest")
                    continue
                if not _media_matches(artifact["path"], declared, bytes(sample)):
                    errors.append(f"artifact[{index}] media type does not match its bytes")
                    continue
                content = _loads_strict_json(json_bytes) if json_bytes is not None else None
                if artifact["role"] in _STRUCTURED_ROLES and content is None:
                    errors.append(f"artifact[{index}] evidence role requires JSON content")
                    continue
                verified[artifact_id] = VerifiedArtifact(
                    artifact_id, artifact["role"], digest.hexdigest(), content
                )
            except (OSError, UnicodeError, ValueError, RecursionError):
                errors.append(f"artifact[{index}] bytes or media content are invalid")
            finally:
                if fd >= 0:
                    os.close(fd)
    finally:
        os.close(root_fd)
    return verified, errors, warnings

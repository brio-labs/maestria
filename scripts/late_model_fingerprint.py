#!/usr/bin/env python3
"""Validate and fingerprint the frozen local mLateOn export.

The command is intentionally side-effect free except for the requested JSON
report. It never downloads model files and refuses bytes that do not match the
profile's immutable artifact hashes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import sys
from pathlib import Path
from typing import Any

PROFILE_PATH = Path(__file__).with_name("late_model_profiles.json")
ARTIFACT_HASH_PREFIX = "sha256:"


def sha256_bytes(data: bytes) -> str:
    return ARTIFACT_HASH_PREFIX + hashlib.sha256(data).hexdigest()


def length_prefixed(parts: list[bytes]) -> bytes:
    output = bytearray()
    for part in parts:
        output.extend(len(part).to_bytes(8, "big"))
        output.extend(part)
    return bytes(output)


def read_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"expected object: {path}")
    return value


def artifact_hash(profile: dict[str, Any], model_dir: Path) -> tuple[str, dict[str, str], dict[str, bytes]]:
    entries = profile.get("artifacts")
    if not isinstance(entries, list) or not entries:
        raise ValueError("profile must list artifacts")
    parsed: list[tuple[str, str, bytes]] = []
    for entry in entries:
        if not isinstance(entry, dict):
            raise ValueError("profile artifacts must be objects")
        relative = entry.get("path")
        expected = entry.get("sha256")
        if not isinstance(relative, str) or not relative or Path(relative).is_absolute():
            raise ValueError("artifact paths must be relative non-empty strings")
        if not isinstance(expected, str) or len(expected) != 64:
            raise ValueError(f"invalid pinned hash for {relative}")
        path = model_dir / relative
        data = path.read_bytes()
        actual = hashlib.sha256(data).hexdigest()
        if actual != expected:
            raise ValueError(f"artifact hash mismatch for {relative}: expected {expected}, got {actual}")
        parsed.append((relative, actual, data))
    parsed.sort(key=lambda item: item[0])
    material = [b"late-interaction-artifact-hash-v1", len(parsed).to_bytes(4, "big")]
    for relative, digest, _ in parsed:
        material.extend((relative.encode("utf-8"), bytes.fromhex(digest)))
    digest = ARTIFACT_HASH_PREFIX + hashlib.sha256(length_prefixed(material)).hexdigest()
    return digest, {relative: digest for relative, digest, _ in parsed}, {relative: data for relative, _, data in parsed}


def vocabulary_hash(tokenizer: dict[str, Any]) -> tuple[str, int]:
    entries: dict[int, str] = {}
    model = tokenizer.get("model")
    vocab = model.get("vocab") if isinstance(model, dict) else None
    if not isinstance(vocab, dict):
        raise ValueError("tokenizer model.vocab is missing")
    for token, token_id in vocab.items():
        if not isinstance(token, str) or not isinstance(token_id, int) or token_id < 0:
            raise ValueError("tokenizer vocabulary contains an invalid entry")
        prior = entries.get(token_id)
        if prior is not None and prior != token:
            raise ValueError(f"vocabulary id collision at {token_id}")
        entries[token_id] = token
    added = tokenizer.get("added_tokens", [])
    if not isinstance(added, list):
        raise ValueError("tokenizer added_tokens must be a list")
    for item in added:
        if not isinstance(item, dict):
            raise ValueError("tokenizer added token must be an object")
        token = item.get("content")
        token_id = item.get("id")
        if not isinstance(token, str) or not isinstance(token_id, int) or token_id < 0:
            raise ValueError("tokenizer added token is invalid")
        prior = entries.get(token_id)
        if prior is not None and prior != token:
            raise ValueError(f"added vocabulary id collision at {token_id}")
        entries[token_id] = token
    material: list[bytes] = [b"late-interaction-vocabulary-v1"]
    for token_id, token in sorted(entries.items()):
        material.append(token_id.to_bytes(8, "big"))
        material.append(token.encode("utf-8"))
    return ARTIFACT_HASH_PREFIX + hashlib.sha256(length_prefixed(material)).hexdigest(), len(entries)


def template_hash(profile: dict[str, Any], kind: str) -> str:
    prefix = profile["query_prefix"] if kind == "query" else profile["document_prefix"]
    payload = {
        "algorithm": "late-interaction-template-v1",
        "kind": kind,
        "prefix": prefix,
        "placement": "once_before_tokenizer_output",
        "native_limit": profile["native_query_token_limit"] if kind == "query" else profile["native_document_token_limit"],
        "request_limit": profile["query_max_vectors"] if kind == "query" else profile["document_max_vectors"],
        "truncate": profile["truncate_direction"],
        "retain_special_tokens": profile["retain_special_tokens"],
        "drop_masked_positions": profile["drop_masked_positions"],
        "pad_token_id": profile["pad_token_id"],
        "query_expansion": profile["query_expansion"],
        "lowercase": profile["lowercase"],
        "punctuation_pruning": profile["punctuation_pruning"],
    }
    encoded = json.dumps(payload, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return sha256_bytes(encoded)


def runtime_versions() -> dict[str, str | None]:
    versions: dict[str, str | None] = {
        "python": platform.python_version(),
        "onnxruntime": None,
        "tokenizers": None,
        "numpy": None,
    }
    for name in ("onnxruntime", "tokenizers", "numpy"):
        try:
            module = __import__(name)
            versions[name] = getattr(module, "__version__", "unknown")
        except ImportError:
            pass
    return versions


def fingerprint(profile_name: str, model_dir: Path, profiles_path: Path) -> dict[str, Any]:
    document = read_json(profiles_path)
    profiles = document.get("profiles")
    profile = profiles.get(profile_name) if isinstance(profiles, dict) else None
    if not isinstance(profile, dict):
        raise ValueError(f"unknown late model profile: {profile_name}")
    config_path = model_dir / "onnx_config.json"
    tokenizer_path = model_dir / "tokenizer.json"
    config = read_json(config_path)
    tokenizer = read_json(tokenizer_path)
    aggregate, file_hashes, artifacts = artifact_hash(profile, model_dir)
    tokenizer_digest, vocabulary_size = vocabulary_hash(tokenizer)
    if config.get("embedding_dim") != profile["output_dimensions"]:
        raise ValueError("ONNX configuration dimension does not match profile")
    if config.get("query_prefix_id") != profile["query_prefix"]["id"]:
        raise ValueError("query prefix id does not match profile")
    if config.get("document_prefix_id") != profile["document_prefix"]["id"]:
        raise ValueError("document prefix id does not match profile")
    result = dict(profile)
    result["artifact_hash"] = aggregate
    result["artifact_file_hashes"] = file_hashes
    result["tokenizer_hash"] = tokenizer_digest
    result["vocabulary_hash"], result["vocabulary_size"] = tokenizer_digest, vocabulary_size
    result["query_template_hash"] = template_hash(profile, "query")
    result["document_template_hash"] = template_hash(profile, "document")
    result["runtime"] = dict(profile["runtime"])
    result["runtime"].update(runtime_versions())
    result["cpu"] = platform.processor() or platform.machine()
    result["onnx_providers"] = ["CPUExecutionProvider"]
    result["profile_name"] = profile_name
    result["profile_schema_version"] = document.get("schema_version")
    result["model_config"] = {
        "model_type": config.get("model_type"),
        "model_class": config.get("model_class"),
        "embedding_dim": config.get("embedding_dim"),
        "pad_token_id": config.get("pad_token_id"),
        "mask_token_id": config.get("mask_token_id"),
        "native_query_token_limit": config.get("query_length"),
        "native_document_token_limit": config.get("document_length"),
    }
    # Keep raw file bytes out of the report. The local export itself is the
    # immutable input; hashes and this complete derived profile are the report identity.
    del artifacts
    return result


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--model-dir", required=True, type=Path)
    parser.add_argument("--profiles", default=PROFILE_PATH, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        report = fingerprint(args.profile, args.model_dir, args.profiles)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"late model fingerprint failed: {error}", file=sys.stderr)
        return 2
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

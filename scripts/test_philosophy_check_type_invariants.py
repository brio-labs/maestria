"""Tests for the philosophy_check type invariants family."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from philosophy_check import shared, type_invariants
from philosophy_check_testbase import PhilosophyCheckFixture


class TypeInvariantsTests(PhilosophyCheckFixture):

    def test_type_invariant_scan_rejects_opposite_boolean_states(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = (
                root / "crates" / "kernel" / "maestria-domain" / "src" / "approval.rs"
            )
            source.parent.mkdir(parents=True)
            source.write_text(
                "pub struct Approval<T: Marker<Vec<u8>>> {\n"
                "    pub is_approved: bool, // misleading { and , tokens\n"
                '    #[serde(rename = "denied{,")]\n'
                "    pub denied: bool,\n"
                "    pub marker: PhantomData<T>,\n"
                "}\n"
                "pub const fn resolve(approved: bool, denied: bool) {}\n",
                encoding="utf-8",
            )

            self.assertEqual(
                type_invariants.scan_type_invariant_modeling(),
                [
                    "crates/kernel/maestria-domain/src/approval.rs struct `Approval` "
                    "represents opposite states `approved` and `denied` as booleans; "
                    "use an enum",
                    "crates/kernel/maestria-domain/src/approval.rs function `resolve` "
                    "accepts opposite states `approved` and `denied` as booleans; "
                    "accept an enum",
                ],
            )

    def test_type_invariant_scan_rejects_boolean_with_optional_state_payload(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "job.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "pub struct Job { pub failed: bool, pub error: Option<String> }\n"
                "pub fn finish(failed: bool, error: Option<String>) {}\n",
                encoding="utf-8",
            )

            self.assertEqual(
                type_invariants.scan_type_invariant_modeling(),
                [
                    "crates/kernel/maestria-domain/src/job.rs struct `Job` coordinates "
                    "boolean state `failed` with optional payload `error`; put the "
                    "payload on an enum variant",
                    "crates/kernel/maestria-domain/src/job.rs function `finish` "
                    "coordinates boolean state `failed` with optional payload "
                    "`error`; accept an enum carrying the payload",
                ],
            )

    def test_type_invariant_scan_rejects_stringly_state(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "task.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "pub struct Task { pub status: String, pub title: String }\n"
                "pub const fn transition(status: &str) {}\n",
                encoding="utf-8",
            )

            self.assertEqual(
                type_invariants.scan_type_invariant_modeling(),
                [
                    "crates/kernel/maestria-domain/src/task.rs struct `Task` "
                    "represents state field `status` as `String`; use an enum or "
                    "validated domain type",
                    "crates/kernel/maestria-domain/src/task.rs function `transition` "
                    "accepts state parameter `status` as `&str`; accept an enum or "
                    "validated domain type",
                ],
            )

    def test_type_invariant_scan_rejects_swappable_primitive_ids(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = (
                root / "crates" / "kernel" / "maestria-domain" / "src" / "relation.rs"
            )
            source.parent.mkdir(parents=True)
            source.write_text(
                "pub struct Relation { pub source_id: u64, pub target_id: u64 }\n"
                "pub fn connect(parent_id: u64, child_id: u64) {}\n"
                'extern "C" fn link(left_id: u64, right_id: u64) {}\n',
                encoding="utf-8",
            )

            self.assertEqual(
                type_invariants.scan_type_invariant_modeling(),
                [
                    "crates/kernel/maestria-domain/src/relation.rs struct `Relation` "
                    "has swappable primitive identities `source_id`, `target_id` of "
                    "type `u64`; use distinct ID types",
                    "crates/kernel/maestria-domain/src/relation.rs function `connect` "
                    "accepts swappable primitive identities `parent_id`, `child_id` "
                    "of type `u64`; use distinct ID types",
                    "crates/kernel/maestria-domain/src/relation.rs function `link` "
                    "accepts swappable primitive identities `left_id`, `right_id` "
                    "of type `u64`; use distinct ID types",
                ],
            )

    def test_type_invariant_scan_accepts_independent_flags_and_typed_ids(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = (
                root / "crates" / "kernel" / "maestria-domain" / "src" / "search.rs"
            )
            source.parent.mkdir(parents=True)
            source.write_text(
                "pub struct QueryId(u64);\n"
                "pub struct CorpusId(u64);\n"
                "pub struct SearchOptions {\n"
                "    pub include_archived: bool,\n"
                "    pub preserve_seed: bool,\n"
                "    pub query_hint: Option<String>,\n"
                "}\n"
                "pub enum SearchState { Planned, Running, Complete }\n"
                "pub fn search(query_id: QueryId, corpus_id: CorpusId) {}\n"
                "fn format_label(kind: &str) {}\n",
                encoding="utf-8",
            )

            self.assertEqual(type_invariants.scan_type_invariant_modeling(), [])

    def test_domain_untyped_json_scan_rejects_value_hole(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                'let value: serde_json::Value = serde_json::json!({"a": 1});\n',
                encoding="utf-8",
            )

            self.assertEqual(
                type_invariants.scan_domain_untyped_json(),
                [
                    "crates/kernel/maestria-domain/src/lib.rs "
                    "uses untyped serde_json::Value in domain source"
                ],
            )

    def test_strategy_field_is_stringly_state(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "trace.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("pub struct Expansion { pub strategy: String }\n")
            violations = type_invariants.scan_type_invariant_modeling()
            self.assertTrue(
                any("state field `strategy`" in item for item in violations), violations
            )

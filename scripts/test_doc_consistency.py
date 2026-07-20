from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("doc-consistency-check.py")
SPEC = importlib.util.spec_from_file_location("doc_consistency_check", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("unable to load doc-consistency-check.py")
DOC_CONSISTENCY_CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DOC_CONSISTENCY_CHECK)


class DocConsistencyTests(unittest.TestCase):
    """Test that the doc consistency checker correctly identifies coverage."""

    def test_all_cli_commands_extracted(self) -> None:
        """Verify that every top-level and nested command is discovered."""
        cli_text = DOC_CONSISTENCY_CHECK.CLI_TYPES.read_text()
        tree = DOC_CONSISTENCY_CHECK.extract_top_level_commands(cli_text)

        # Top-level commands
        self.assertIn("Init", tree)
        self.assertIn("Index", tree)
        self.assertIn("Search", tree)
        self.assertIn("OpenEvidence", tree)
        self.assertIn("Evidence", tree)
        self.assertIn("Status", tree)
        self.assertIn("Doctor", tree)
        self.assertIn("Start", tree)
        self.assertIn("Task", tree)
        self.assertIn("Memory", tree)
        self.assertIn("Approval", tree)

        # Index subcommands
        self.assertIn("Generations", tree["Index"])
        self.assertIn("Repository", tree["Index"])

        # Search subcommands
        self.assertIn("Explain", tree["Search"])
        self.assertIn("Trace", tree["Search"])
        self.assertIn("Compare", tree["Search"])
        self.assertIn("Code", tree["Search"])

        # Code subcommands (nested under Search > Code)
        self.assertIn("Symbol", tree["Search"]["Code"])
        self.assertIn("Path", tree["Search"]["Code"])
        self.assertIn("Regex", tree["Search"]["Code"])
        self.assertIn("Context", tree["Search"]["Code"])

        # Task subcommands
        self.assertIn("Start", tree["Task"])
        self.assertIn("Show", tree["Task"])
        self.assertIn("AddEvidence", tree["Task"])
        self.assertIn("RequestValidation", tree["Task"])
        self.assertIn("Complete", tree["Task"])

        # Memory subcommands
        self.assertIn("Candidates", tree["Memory"])
        self.assertIn("Propose", tree["Memory"])
        self.assertIn("Promote", tree["Memory"])

        # Approval subcommands
        self.assertIn("List", tree["Approval"])
        self.assertIn("Resolve", tree["Approval"])

    def test_readme_has_full_coverage(self) -> None:
        """Verify that the current README documents every CLI command."""
        cli_text = DOC_CONSISTENCY_CHECK.CLI_TYPES.read_text()
        readme_text = DOC_CONSISTENCY_CHECK.README.read_text()
        tree = DOC_CONSISTENCY_CHECK.extract_top_level_commands(cli_text)
        missing = DOC_CONSISTENCY_CHECK.find_readme_gaps(readme_text, tree)
        self.assertEqual(
            missing, [],
            f"README.md is missing documentation for: {missing}"
        )

    def test_camel_to_kebab_conversion(self) -> None:
        """Verify CamelCase to kebab-case conversion."""
        cases = {
            "Init": "init",
            "OpenEvidence": "open-evidence",
            "AddEvidence": "add-evidence",
            "RequestValidation": "request-validation",
            "CodeSearchCommands": "code-search-commands",
            "CliTaskPriority": "cli-task-priority",
        }
        for camel, expected in cases.items():
            actual = DOC_CONSISTENCY_CHECK._camel_to_kebab(camel)
            self.assertEqual(actual, expected,
                             f"_camel_to_kebab('{camel}') = '{actual}', "
                             f"expected '{expected}'")

    def test_find_readme_gaps_reports_missing(self) -> None:
        """Verify that gaps are detected in a minimal readme."""
        minimal_readme = "# Test readme\n"
        tree = {"Init": None, "MissingCmd": None}
        gaps = DOC_CONSISTENCY_CHECK.find_readme_gaps(minimal_readme, tree)
        self.assertIn("`init`", gaps,
                      "should report missing top-level command")
        self.assertIn("`missing-cmd`", gaps,
                      "should report missing CamelCase command")

    def test_find_readme_gaps_detects_heading(self) -> None:
        """Verify that a heading is recognised as documentation."""
        readme_with_heading = "# Test\n### `init`\nSome text\n"
        tree = {"Init": None}
        gaps = DOC_CONSISTENCY_CHECK.find_readme_gaps(readme_with_heading, tree)
        self.assertNotIn("`init`", gaps,
                         "heading should satisfy documentation check")

    def test_find_readme_gaps_detects_usage(self) -> None:
        """Verify that a usage line is recognised as documentation."""
        readme_with_usage = (
            "# Test\n\n"
            "```\n"
            "maestria init -i .maestria-dev\n"
            "```\n"
        )
        tree = {"Init": None}
        gaps = DOC_CONSISTENCY_CHECK.find_readme_gaps(readme_with_usage, tree)
        self.assertNotIn("`init`", gaps,
                         "usage example should satisfy documentation check")

    def test_nested_commands_detected_with_deep_usage(self) -> None:
        """Verify that a deep usage (with flags between words) is detected."""
        readme = (
            "# Test\n\n"
            "```\n"
            "maestria index -i .maestria-dev repository ~/Projects\n"
            "```\n"
        )
        tree = {"Index": {"Repository": None}}
        gaps = DOC_CONSISTENCY_CHECK.find_readme_gaps(readme, tree)
        self.assertNotIn("`index repository`", gaps,
                         "deep usage with flags should satisfy check")


    def test_daemon_documentation_covers_operations_and_limits(self) -> None:
        protocol = """pub enum ClientOperation {
    Status,
    Search { query: String },
    Evidence { evidence_id: u64 },
    Task,
    ModelAgentPropose { proposal: Payload },
}"""
        api = "pub(crate) const MAX_REQUEST_BYTES: usize = 65536;"
        server = "timeout(Duration::from_secs(5), read_request_line(stream))"
        operations = (
            "`status` `search` `evidence` `task` `model_agent_propose`"
        )
        docs = f"{operations} 64 KiB five-second"

        self.assertEqual(
            DOC_CONSISTENCY_CHECK.find_daemon_documentation_gaps(
                protocol, api, server, operations, docs
            ),
            [],
        )
        self.assertIn(
            "daemon operation `model_agent_propose` missing from README.md",
            DOC_CONSISTENCY_CHECK.find_daemon_documentation_gaps(
                protocol, api, server, "`status`", docs
            ),
        )

if __name__ == "__main__":
    unittest.main()

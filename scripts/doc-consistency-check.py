#!/usr/bin/env python3
"""
Documentation consistency checker.

Derives every CLI command and subcommand name from cli_types.rs,
then verifies that the README documents them. Only flags genuine
coverage gaps — not incidental prose matches.

Exit code 0 on full coverage, 1 on any gap.
"""

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CLI_TYPES = REPO_ROOT / "crates" / "apps" / "maestria-cli" / "src" / "cli_types.rs"
README = REPO_ROOT / "README.md"


def find_matching_brace(text: str, start: int) -> int:
    """Return the position just past the matching '}'. `text[start]` must be '{'."""
    depth = 1
    i = start + 1
    while i < len(text) and depth > 0:
        if text[i] == '{':
            depth += 1
        elif text[i] == '}':
            depth -= 1
        i += 1
    return i


def extract_enum_body(text: str, enum_name_start: int) -> str:
    """Extract the brace-delimited body of an enum starting at `enum_name_start`."""
    brace_pos = text.index('{', enum_name_start)
    end = find_matching_brace(text, brace_pos)
    return text[brace_pos + 1:end - 1]


def _enum_pattern(enum_name: str) -> str:
    return r'pub enum ' + re.escape(enum_name) + r'\s*\{'


def _camel_to_kebab(name: str) -> str:
    """Convert CamelCase to kebab-case (e.g. OpenEvidence -> open-evidence)."""
    parts = []
    for i, ch in enumerate(name):
        if ch.isupper() and i > 0:
            parts.append('-')
        parts.append(ch.lower())
    return ''.join(parts)


def extract_top_level_commands(text: str) -> dict:
    """Extract the structured tree of top-level Commands enum."""
    m = re.search(_enum_pattern("Commands"), text)
    if not m:
        return {}
    body = extract_enum_body(text, m.start())

    tree = {}
    variant_re = re.compile(r'^\s+(\w+)\s*[{(;]', re.MULTILINE)
    for vm in variant_re.finditer(body):
        name = vm.group(1)
        if name == "Cli":
            continue
        if name in ("Search", "Index", "Task", "Evidence", "Memory", "Approval"):
            enum_field = name + "Commands"
            tree[name] = _extract_subcommand_children(text, enum_field, name)
        else:
            tree[name] = None
    return tree


def _extract_subcommand_children(text: str, enum_name: str, parent_cmd: str) -> dict:
    """Extract children of a subcommand enum, handling nesting (e.g. Search > Code)."""
    m = re.search(_enum_pattern(enum_name), text)
    if not m:
        return {}

    body = extract_enum_body(text, m.start())
    children = {}

    variant_re = re.compile(r'^\s+(\w+)\s*[{(;]', re.MULTILINE)
    for vm in variant_re.finditer(body):
        name = vm.group(1)
        if name in ("Cli", "Commands", "SearchCommands", "CodeSearchCommands",
                    "IndexCommands", "EvidenceCommands", "TaskCommands",
                    "MemoryCommands", "ApprovalCommands", "CliTaskPriority",
                    "ClientOperation"):
            continue
        sub_pat = (r'^\s+' + re.escape(name) +
                   r'\s+\{[^}]*command\s*:\s*CodeSearchCommands')
        if re.search(sub_pat, body, re.MULTILINE):
            children[name] = _extract_subcommand_children(text, "CodeSearchCommands", name)
        elif parent_cmd == "Search" and name == "Code":
            children[name] = _extract_subcommand_children(text, "CodeSearchCommands", name)
        else:
            children[name] = None
    return children


def find_readme_gaps(readme_text: str, tree: dict, prefix_words: list[str] | None = None) -> list[str]:
    """Check which commands from the tree are missing from README."""
    if prefix_words is None:
        prefix_words = []

    missing = []

    for cmd, children in tree.items():
        cmd_lower = cmd.lower()
        cmd_kebab = _camel_to_kebab(cmd)

        has_heading = False
        has_usage = False

        for variant in [cmd_lower, cmd_kebab]:
            cmd_esc = re.escape(variant)

            # Check for heading: ### `init` or #### `task start`
            if not has_heading:
                heading_pattern = r'#{3,6}\s+`' + cmd_esc + r'`'
                has_heading = bool(re.search(heading_pattern, readme_text, re.IGNORECASE))

            # Check for usage like `maestria index generations` in a code block.
            # Code blocks use fenced backticks, so no leading backtick on each line.
            # We check two variants:
            #   1) Simple: `maestria <prefix> <cmd>` with no flags between words.
            #   2) Deep: allows optional flags/args between command words, so
            #      `maestria index -i .maestria-dev repository <path>` matches
            #      `index repository`.
            if not has_usage:
                esc_parts = [re.escape(_camel_to_kebab(w)) for w in prefix_words]
                esc_parts.append(cmd_esc)
                usage_pattern = (r'maestria\s+' + r'\s+'.join(esc_parts) +
                                 r'(?:\s|`|$|\.|,|;|\)|\|)')
                has_usage = bool(re.search(usage_pattern, readme_text))
            if not has_usage:
                esc_parts_deep = []
                for w in prefix_words:
                    esc_parts_deep.append(re.escape(_camel_to_kebab(w)))
                    # Allow optional flags/args between command words.
                    # Includes \s+ so `index -i .maestria-dev repository` matches.
                    esc_parts_deep.append(r'(?:\s+-\S+(?:\s+\S+)*\s+)?')
                esc_parts_deep.append(cmd_esc)
                joined = ''.join(esc_parts_deep)
                usage_pattern_deep = r'maestria\s+' + joined + r'(?:\s|`|$|\.|,|;|\)|\|)'
                has_usage = bool(re.search(usage_pattern_deep, readme_text))
        documented = has_heading or has_usage
        if not documented:
            display_words = prefix_words + [cmd]
            display_name = ' '.join(_camel_to_kebab(w) for w in display_words)
            missing.append('`' + display_name + '`')

        if children:
            new_prefix = prefix_words + [cmd]
            missing.extend(find_readme_gaps(readme_text, children, new_prefix))

    return missing


def main() -> int:
    cli_text = CLI_TYPES.read_text()
    readme_text = README.read_text()

    tree = extract_top_level_commands(cli_text)

    print("=== Command tree extracted from cli_types.rs ===")
    _print_tree(tree, 0)

    missing = find_readme_gaps(readme_text, tree)

    if missing:
        print("\n=== MISSING FROM README ===")
        for path in missing:
            print("  " + path)
        print("\n" + str(len(missing)) + " commands or subcommands lack documentation.")
        return 1

    print("\nAll commands and subcommands are documented in README.md.")
    return 0


def _print_tree(tree, indent=0):
    prefix = "  " * indent
    if isinstance(tree, dict):
        for k, v in tree.items():
            if isinstance(v, dict) and v:
                print(prefix + "- " + k + "/")
                _print_tree(v, indent + 1)
            else:
                print(prefix + "- " + k)
    else:
        print(prefix + str(tree))


if __name__ == "__main__":
    sys.exit(main())

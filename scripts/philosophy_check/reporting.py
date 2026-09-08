"""Reporting rule family."""

from __future__ import annotations

from . import shared
from .contract_tests import (
    scan_documentation_contract,
    scan_responsibility_maps,
)
from .dependency_graph import (
    scan_facade_boundaries,
    scan_kernel_dependency_closure,
    scan_kernel_imports,
    scan_kernel_manifests,
    scan_kernel_sources,
)
from .formatting import (
    scan_cohesion,
    scan_exemption_expiry,
    scan_function_sizes,
    scan_mixed_responsibilities,
    scan_module_sizes,
    scan_readability_style,
)
from .panic_and_lint import (
    scan_bypassable_validation,
    scan_cancellation_docs,
    scan_debug_output,
    scan_env_mutation,
    scan_failure_tokens,
    scan_generated_blobs,
    scan_kernel_interior_mutability,
    scan_markers,
    scan_memory_unsafety_markers,
    scan_process_exit,
    scan_production_asserts,
    scan_rust_forbidden_methods,
    scan_rust_lint_bypasses,
    scan_string_typed_errors,
    scan_unbounded_channels,
    scan_unchecked_apis,
)
from .secrets import (
    scan_hardcoded_secrets,
)
from .type_invariants import (
    scan_domain_untyped_json,
    scan_type_invariant_modeling,
)

def main() -> int:
    violations = []
    marker_violations = scan_markers()
    violations.extend(
        f"{path} contains forbidden task marker" for path in marker_violations
    )
    violations.extend(scan_kernel_manifests())
    violations.extend(scan_kernel_dependency_closure())
    violations.extend(scan_kernel_imports())
    violations.extend(scan_kernel_sources())
    violations.extend(scan_type_invariant_modeling())
    violations.extend(scan_bypassable_validation())
    violations.extend(scan_documentation_contract())
    violations.extend(scan_responsibility_maps())
    violations.extend(scan_module_sizes())
    violations.extend(scan_exemption_expiry())
    violations.extend(
        f"{path} contains a Rust lint-bypass attribute"
        for path in scan_rust_lint_bypasses()
    )
    violations.extend(scan_unbounded_channels())
    violations.extend(scan_rust_forbidden_methods())
    violations.extend(scan_memory_unsafety_markers())
    violations.extend(scan_unchecked_apis())
    violations.extend(scan_failure_tokens())
    violations.extend(scan_process_exit())
    violations.extend(scan_env_mutation())
    violations.extend(scan_debug_output())
    violations.extend(scan_generated_blobs())
    violations.extend(scan_string_typed_errors())
    violations.extend(scan_hardcoded_secrets())
    violations.extend(scan_cancellation_docs())
    violations.extend(scan_kernel_interior_mutability())
    violations.extend(scan_production_asserts())
    violations.extend(scan_domain_untyped_json())
    violations.extend(scan_facade_boundaries())
    violations.extend(scan_cohesion())
    violations.extend(scan_readability_style())
    violations.extend(scan_function_sizes())
    violations.extend(scan_mixed_responsibilities())

    # Collapse exact duplicates across scans (e.g. domain sources are kernel
    # sources too) while preserving the first occurrence's ordering.
    seen = set()
    unique_violations = []
    for violation in violations:
        if violation not in seen:
            seen.add(violation)
            unique_violations.append(violation)
    violations = unique_violations

    if violations:
        print("philosophy-check failed:")
        for violation in violations:
            print(f" - {violation}")
        return 1

    print("philosophy-check passed")
    return 0

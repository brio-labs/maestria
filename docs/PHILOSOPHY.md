# Maestria Philosophy

This document defines enforceable architecture rules for bootstrap and later phases.

## Rules

1. Domain code must be deterministic: no time, network, filesystem, or process calls.
2. All I/O-capable work must be represented as explicit effect values and executed by runtime.
3. Evidence must carry provenance (path/url, range, snapshot, timestamps).
4. Every factual answer path should be auditable through event, command, or evidence trail.
5. The repo must maintain a conservative, local-first baseline; remote services are adapters.
6. `TODO` and `FIXME` markers are not allowed in source, config, or docs.

## Enforcement

- `scripts/philosophy-check.py`
- Workspace lint and test gates in CI
- Contract checks for kernel inputs/outputs and transitions

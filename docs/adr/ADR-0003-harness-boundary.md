# ADR-0003: Harness Boundary

## Context

The harness executes scoped actions for tools, commands, and browser sessions.

## Decision

Keep harness behind contract interfaces and never allow direct tool execution from domain.

## Consequences

- Auditable and testable side effects.
- Consistent policy checks before action.
- Clear replacement path across execution backends.

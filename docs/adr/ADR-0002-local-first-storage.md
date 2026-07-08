# ADR-0002: Local-First Storage

## Context

Long-term trust in an AI operating layer requires resilient local truth.

## Decision

Use local persisted stores as the system of record. Treat remote services as adapters.

## Consequences

- Strong provenance for local and remote evidence.
- Reduced risk if upstream providers are unavailable.
- Offline-first behavior is a default, not an exception.

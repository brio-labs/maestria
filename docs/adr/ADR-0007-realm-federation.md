# ADR-0007: Local Realm Federation

## Context

Maestria instances are intentionally isolated: each owns its manifest, source
scope, database, blob store, index projections, and daemon credential. A useful
cross-instance read must not turn that isolation into shared files, a shared
database, or reuse of a provider daemon token. Those alternatives bypass the
provider's source policy and make revocation, provenance, and audit incomplete.

The first federation surface needs a durable provider decision that can be
revoked, a bounded consumer capability, and authorization before any provider
candidate is retrieved or scored.

## Decision

Use local, provider-owned, read-only realm federation.

- Every schema-v2 instance manifest owns one stable `RealmId`; existing
  schema-v1 manifests require the explicit `maestria realm migrate` boundary.
- A provider issues at most one active realm-read grant for a consumer realm.
  The grant stores only a domain-separated digest of a randomly generated
  bearer credential, access (`search-only` or `search-and-open-evidence`), a
  sensitivity ceiling, and finite result/evidence-byte bounds.
- Grant issuance, revocation, and successful federated accesses are append-only
  domain events. The SQLite grant table is a rebuildable current-state
  projection, not the authority.
- The provider daemon authenticates a federation request with the consumer
  realm and bearer credential, reloads the current grant, derives its digest,
  applies grant and provider policy, then retrieves. The provider daemon token
  is never sent to another instance.
- Consumer bindings are private local files containing the provider Unix socket
  path and credential. Federation remains Unix-socket-only; it does not create
  TCP, HTTP, filesystem, blob-store, database, index, task, memory, or mutation
  sharing.
- Provider retrieval uses the composed authorization context in every enabled
  non-graph lane. Graph expansion is disabled until the graph port can enforce
  authorization before relation materialization. Provider replies preserve
  provider realm and evidence provenance, are bounded, and are audited only
  after their operation succeeds.

## Consequences

- Revocation takes effect for the next provider request, including after daemon
  restart; stale consumer bindings become inert.
- A `search-only` capability cannot open evidence. A search-and-open-evidence
  capability receives no more than the grant's bounded excerpt.
- Federation has no per-user or per-agent principal in v1. The consumer realm
  is the authenticated actor; adding principals requires a distinct identity
  and grant model rather than overloading `RealmId`.
- A provider can report an unavailable/degraded federated search rather than
  execute a graph lane with post-retrieval filtering.
- Operators must run both local daemons and explicitly create, install, list,
  and revoke grants. Federation never changes ordinary local `search` or
  `open-evidence` behavior.

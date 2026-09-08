# ADR-0010: The Daemon Stays Socket-Only; Studio Remains the Browser Proxy

## Status

Proposed. Decision pending review; recorded before further Studio API
endpoints are added.

## Context

Every browser request today crosses two servers and two auth surfaces:

```text
browser → Studio (axum: rust-embed assets, REST, origin+bearer middleware)
        → daemon (newline-delimited JSON over the per-instance Unix socket)
        → daemon services
```

Each Studio HTTP handler is a thin translation of a REST call into one
typed `ClientOperation` over the socket (`maestria-studio/src/http/*.rs`),
and the daemon independently authenticates the per-instance token and
checks read/write scope on every operation (rule 48; `daemon.token`,
socket permissions `0o600`). The CLI spawns the Studio server per session
(`maestria studio -i <instance>`), separate from the long-lived daemon.

The simplicity/performance review flagged the axum + hyper + tower stack in
Studio as the largest remaining async dependency cluster and asked whether
the proxy topology should be consolidated: the daemon serving REST and
embedded assets directly over HTTP, with Studio shrunk to a launcher.

Constraints: rule 48 (per-instance token auth and scope on the daemon
boundary), rule 32 (no direct database writes from CLI/API handlers),
rule 1 (deterministic domain, I/O confined to adapters), and the security
posture in `docs/SECURITY.md` that the daemon has no network-accessible
transport. No measured latency problem exists at the IPC hop: socket
round-trips are microseconds against search budgets of hundreds of
milliseconds.

## Decision

Keep the two-server topology. The daemon stays socket-only; Studio remains
the single browser-facing HTTP surface. The contract between them is
codified as follows:

- **The daemon never serves HTTP, WebSocket, or any network transport.**
  Its only endpoint is the per-instance Unix domain socket with token
  authentication, scope checks, and `0o600` permissions. This is a
  security invariant, not an implementation accident.
- **Studio owns browser-facing concerns**: static UI assets, the REST
  shape, origin enforcement, bearer-token checks, and request
  shaping/limits. Studio holds no durable state and no database access;
  it is a stateless presentation server that can be restarted or replaced
  without touching stored data.
- **The daemon owns capability enforcement**: every `ClientOperation` is
  token-authenticated and scope-checked in the daemon. Studio's
  middleware is browser-boundary hygiene, never the authority; a
  compromised browser session must not widen into socket-level access.
- **The extension point is the typed socket API.** A browser need maps to
  an existing `ClientOperation`, or a new one is added to the daemon API
  with its own scope classification and a thin Studio handler. REST
  endpoints are never added without a corresponding typed operation.
- **UI lifecycle stays decoupled from the daemon.** The Studio server is
  spawned and restarted independently (`maestria studio`); daemon uptime
  is data-plane uptime and does not bundle presentation-layer restarts.

## Alternatives considered

**Consolidate — the daemon serves REST and embedded assets directly over
HTTP; Studio shrinks to a launcher or disappears.** One binary, one auth
story, no IPC hop. Rejected: it moves the HTTP attack surface (request
parsing, asset serving, origin handling) into the most privileged,
long-lived process, growing the scope `docs/SECURITY.md` must defend; it
couples UI lifecycle to daemon uptime; and it does not even remove the
async dependency cluster — axum/hyper/tower relocate into the daemon
rather than disappear, so the dependency-weight argument buys relocation,
not elimination. The IPC hop it removes is microseconds against
hundred-millisecond search budgets.

**Thin-client consolidation — drop the server-rendered REST entirely and
have the daemon speak the browser protocol.** Same attack-surface and
lifecycle costs as consolidation, plus a bespoke browser protocol in place
of plain REST. Rejected with the above.

**Keep the proxy, but let Studio touch the database read-only for hot
paths.** Rejected: rule 32 and the trust-zone model keep storage access
behind the daemon; a read-only exception would still widen the browser
boundary into the storage tier and split the enforcement point.

## Consequences

- The async web stack is justified permanently inside `maestria-studio`;
  dependency reviews should weigh it as presentation-layer cost, not
  debt.
- Two processes remain an operations fact (`docs/OPERATIONS.md`): the
  daemon for the instance, the Studio server per UI session.
- Every new browser capability costs a typed `ClientOperation` plus a
  thin handler — slightly more ceremony, deliberately, so capability and
  scope classification stay in one place.
- Revisit triggers for a new ADR: a requirement for daemon HTTP access
  from non-UI local clients, or a measured IPC bottleneck on the socket
  path (none exists today).

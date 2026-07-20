# Daemon Client Boundary

The running daemon exposes a local, read-only client boundary for one Maestria
instance. It is newline-delimited JSON over a Unix domain socket:

```text
<instance>/system/daemon.sock
```

The daemon creates a per-instance credential at:

```text
<instance>/system/daemon.token
```

Both files are private to the account that owns the instance. A client must
possess the token and connect to the matching instance socket. The daemon does
not listen on TCP and does not accept requests without the token.

## Request envelope

Each request is one JSON object followed by a newline:

```json
{
  "token": "<contents of system/daemon.token>",
  "operation": {
    "type": "search",
    "query": "validation gate",
    "limit": 10
  }
}
```

Supported operation tags are:

- `status`
- `search` with `query` and a `limit` from 1 through 100
- `evidence` with `evidence_id`
- `task` with an optional `task_id`
- `model_agent_propose` with a bounded `proposal` payload

The server reads at most 64 KiB per request line and applies a five-second
timeout while reading a request. It also permits at most 32 concurrent
connections.

The response is one typed JSON envelope. Successful responses contain a
`response` object tagged with `status`, `search`, `evidence`, `task`, or
`model_agent_proposal`. Failed requests contain an `error` string and never
mutate domain state.

## Scope and provenance

The boundary is intentionally read-only. Search uses the daemon's read-only
retrieval runtime, including ACL, trust, sensitivity, quarantine, and
prompt-injection filtering. Evidence requests use the core evidence-opening
service, which verifies source snapshots and hides records denied by retrieval
policy. Task and status responses are projections of replayed authoritative
state; they do not write storage.

The supported Rust client is `maestria_daemon::DaemonClient`:

```rust
let client = maestria_daemon::DaemonClient::from_instance(&layout)?;
let response = client
    .request(maestria_daemon::ClientOperation::Status)
    .await?;
```

This boundary keeps transport DTOs separate from domain entities while
preserving stable identifiers, search trace identity, evidence provenance, and
validation-relevant task state.

## Supported model-agent boundary

Model integrations must keep generated plans, claims, rewrites, and memory
proposals outside the domain kernel. The supported typed boundary is
`maestria_ports::ModelAgentProposal`. An adapter validates the bounded query,
search limit, command, capability, timeout, expected index generation, and
source evidence IDs before obtaining a `GovernedAgentProposal`.

Validation is deterministic and rejects stale generations, unknown evidence,
oversized context, unsupported capabilities, and timeouts outside one through
120 seconds. The resulting `HarnessRequest` is still submitted through the
existing runtime governance, scope, approval, effect journal, cancellation,
and stale-feedback checks; the model never invokes a harness adapter directly.
Task completion remains validation-gated, and memory promotion remains
evidence- and policy-gated.

## Model Agent Proposal Endpoint

### `ModelAgentPropose`

Validates and orchestrates a bounded model agent workflow:

1. **Proposal validation** — the endpoint rejects proposals with empty queries,
   queries exceeding 4096 characters, limits outside 1..100, empty commands,
   commands exceeding 4096 characters, unsupported capabilities, timeouts outside
   1..120 seconds, stale generations (expected_generation does not match the
   current index generation), evidence lists exceeding 100 IDs, and unrecognised
   evidence IDs.

2. **Search knowledge** — when the proposal includes a non-empty query, the
   endpoint executes a governed read-only search using the instance's configured
   retrieval runtime and returns evidence candidates.

3. **Governed harness execution** — the endpoint applies shell grammar
   restrictions (only `echo`, `pwd`, `cat`; no shell metacharacters) and scope
   containment before executing the command through the local harness adapter.
   Execution is bounded by the proposal's timeout (1–120 seconds).

4. **Harness outcome evidence** — the harness exit code, stdout, and stderr are
   sent to the runtime as a `HarnessRunCompleted` domain input for durable
   evidence creation.

5. **Validation-gated task completion** — when a task ID is supplied, the
   endpoint evaluates the task against the governance `ValidationGate`.

6. **Evidence/policy-gated memory candidate** — when harness output and
   evidence are present, the endpoint evaluates a memory candidate through the
   governance `MemoryPromotionGate` and sends a `CreateMemoryCandidate` domain
   input to the runtime.

### Security Limitations

- **Per-instance token authentication:** The endpoint reuses the existing
  daemon token authentication. Clients must present the token read from the
  instance system directory (`daemon.token`). Token rotation requires a
  daemon restart.

- **Harness sandboxing:** The local shell harness adapter restricts execution
  to `echo`, `pwd`, and `cat`. Shell metacharacters (`|`, `&`, `;`, `$`, `` ` ``,
  etc.) are rejected before any subprocess is spawned. Scope containment
  limits readable roots and blocks forbidden paths and filename patterns.

- **No network egress:** The daemon endpoint does not serve HTTP, WebSocket,
  or any network-accessible transport. Communication is exclusively over a
  Unix domain socket with file-system permissions set to `0o600`.

- **No remote model adapter:** The model agent proposal type is a local
  boundary. Clients (model adapters, CLI tools, orchestrators) run on the same
  host and communicate over the local Unix socket.

- **Rate limiting:** The daemon API server permits up to 32 concurrent
  connections. The model agent proposal handler itself has no additional
  rate limiting; long-running harness executions block a connection slot for
  the duration of the command.

- **No privilege escalation:** The daemon runs with the privileges of the
  user who started it. Harness subprocesses inherit the same uid/gid with
  no additional sandboxing (seccomp, landlock, or containerisation).

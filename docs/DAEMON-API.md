# Daemon Client Boundary

The running daemon exposes an authenticated client boundary for one Maestria
instance. Read operations are projections of replayed kernel state; notebook
and draft operations are the typed, durable mutation surface used by Studio.
Transport is newline-delimited JSON over a Unix domain socket:

```text
<instance>/system/daemon.sock
```

The daemon creates a per-instance credential at:

```text
<instance>/system/daemon.token
```

Both files are private to the account that owns the instance. Ordinary local
requests must possess the token and connect to the matching instance socket.
Federation requests use the tagged consumer realm grant instead; the daemon
does not listen on TCP.

## Request envelope

Each request is one JSON object followed by a newline. Authentication is
tagged; the instance token and a realm grant are different capabilities.

An ordinary local request uses the owner-only instance token:

```json
{
  "authentication": {
    "type": "instance_token",
    "token": "<contents of system/daemon.token>"
  },
  "operation": {
    "type": "search",
    "query": "validation gate",
    "limit": 10
  }
}
```

A provider receives a federated request only through a consumer binding:

```json
{
  "authentication": {
    "type": "federation_grant",
    "consumer_realm": "<64 lowercase hex realm ID>",
    "credential": "<64 lowercase hex bearer credential>"
  },
  "operation": {
    "type": "federation_search",
    "provider_realm": "<64 lowercase hex realm ID>",
    "query": "validation gate",
    "limit": 10
  }
}
Instance-token operations are `status`, `retrieval_status`, `search`, `evidence`, `task`,
`model_agent_propose`, `model_agent_status`, `model_agent_resolve`,
`realm_grant_create`, `realm_grant_list`, `realm_grant_revoke`,
`install_federation_binding`, and the notebook/draft operations listed below.
A federation credential authorizes only `federation_search` and
`federation_evidence`; it cannot call ordinary local operations, status,
notebook endpoints, task/model-agent endpoints, or grant administration.

Every request and reply is one capped 64 KiB NDJSON frame. The serialized JSON
body **plus its terminating newline** must be at most 64 KiB; a body at the
boundary is accepted and a body whose body-plus-newline exceeds it is rejected.
The server rejects oversized or unterminated frames before extending its
allocation beyond that cap, applies a five-second request-read timeout, and
permits at most 32 concurrent connections. Dropping a client request closes its
socket and stops waiting for its reply; work already accepted by the daemon is
not implicitly rolled back or guaranteed to stop.

The response is one typed JSON envelope. A success has `response` set and both
`error` and `error_code` set to `null`. A failure has `response: null`, a
human-readable string `error`, and a machine-readable `error_code` from:
`unauthorized`, `invalid_input`, `not_found`, `source_unavailable`,
`source_not_selected`, `revision_conflict`, `no_evidence`,
`request_too_large`, `daemon_unavailable`, or `internal`.
`daemon_unavailable` is returned when the socket accepts no connection —
nothing is serving the instance right now (never started, stopped, or
killed). Clients may treat it as "run locally" instead of as a failure.

## Notebook operations

Studio uses the authenticated instance-token operations
`notebook_list`, `notebook_create`, `notebook_get`, `notebook_rename`,
`notebook_delete`, `notebook_source_catalog`, `notebook_source_attach`,
`notebook_source_detach`, `notebook_context`, `notebook_evidence`,
`notebook_draft_list`, `notebook_draft_get`, `notebook_draft_save`, and
`notebook_draft_delete`.

Notebook mutations are durable domain inputs, not direct SQLite or blob-path
writes. `notebook_create`, rename, source attach/detach, and notebook delete
return only after the corresponding event is persisted. Attach is idempotent
for an already-attached key, detach is idempotent for an absent key, and
deleting a notebook removes its live drafts in the same durable transition.
Draft save accepts Markdown and up to 12 unique evidence IDs. A new draft
requires `draft_id: null` and `expected_revision: null`; an update requires
both the draft ID and its exact current revision. A mismatch returns
`revision_conflict` and leaves the submitted body untouched. Blob persistence
is correlated with the event append; a blob write without a committed event
does not expose a draft.

Notebook context always supplies a source-selection filter alongside normal
retrieval authorization. Only currently attached, manifest-allowed, indexed
artifacts may contribute candidates or evidence. The filter is applied before
fusion, reranking, graph expansion, and evidence loading; excluded artifacts
cannot affect scores or coverage. The context response carries a deterministic
source-selection digest. Direct evidence opens for an unselected artifact
return `source_not_selected` without path or excerpt metadata. Saved drafts
retain frozen citation metadata so they can be reopened after a source changes
or disappears.

## Index choice operations

Document indexing (files/directories under a root) and repository code
indexing (a git repository) share the deterministic selection vocabulary
(`Recommended`/`Maybe`/`Noise` classes and per-directory `IndexPolicy`
switches: `max_file_bytes`, `skip_generated`, `skip_minified`) but index
different populations. Both sets require the root to be inside the instance
read scope; client-supplied includes and policy keys are validated for
containment and rejected with `invalid_input` when they escape the root.

### `index_candidates`, `index_selection_get`, `index_selection_save`, `index_run`

`index_candidates` scans a root (a home directory) and returns its bounded
candidate tree. `index_selection_get` loads the persisted document selection
profile (`system/index-selection.json`; `profile: null` when absent);
`index_selection_save` persists a profile after validating every include and
policy key against the profile root. `index_run` indexes the whitelisted
files under a root through the live runtime; it requires the runtime and
reports `submitted`/`skipped` counts.

```json
{"type": "index_candidates", "root": "/home/you"}
{"type": "index_selection_get"}
{"type": "index_selection_save", "profile": {"root": "/home/you", "includes": ["/home/you/docs"], "policies": {}}}
{"type": "index_run", "root": "/home/you", "includes": ["/home/you/docs"], "policies": {}}
```

### `repository_index_candidates`, `repository_index_selection_get`, `repository_index_selection_save`, `repository_index_run`, `repository_index_status`, `repository_index_progress_get`

`repository_index_candidates` scans a repository root (code/document/manifest
population, generated dumps excluded from the walk) and returns its bounded,
classified candidate tree; the root node is always `Recommended`. The
selection operations mirror the document ones but persist to
`system/repository-index-selection.json`, with includes and policy keys
stored repository-relative. `repository_index_run` builds or incrementally
updates the repository code index over exactly the selection (identity,
delta, records, and freshness are all selection-scoped; a selection or policy
change forces a full rebuild), registers every indexed source as a canonical
artifact through the runtime, and reports `mode` (`full`/`incremental`/`noop`),
the full index summary, and `registered`/`skipped` counts. `repository_index_status`
reports whether an index exists for the root, its summary, and its current
freshness verdict.

```json
{"type": "repository_index_candidates", "root": "/home/you/projects/maestria"}
{"type": "repository_index_selection_get"}
{"type": "repository_index_selection_save", "profile": {"root": "/home/you/projects/maestria", "includes": ["crates/one"], "policies": {"crates/one": {"max_file_bytes": 1048576, "skip_generated": false, "skip_minified": true}}}}
{"type": "repository_index_run", "root": "/home/you/projects/maestria", "includes": ["/home/you/projects/maestria/crates/one"], "policies": {}}
{"type": "repository_index_status", "root": "/home/you/projects/maestria"}
{"type": "repository_index_children", "root": "/home/you/projects/maestria", "path": "crates/one"}
{"type": "repository_index_files", "root": "/home/you/projects/maestria", "path": "crates/one"}
{"type": "repository_index_progress_get"}
```

`repository_index_children` returns the bounded candidate subtree of a
repository-relative directory for lazy expansion of the selection tree;
`repository_index_files` lists the direct files of a repository-relative
directory (relative paths, classified by the same policy vocabulary, with a
`truncated` flag when the listing is capped); both validate the path against
the root with `invalid_input` on escape. `repository_index_progress_get` returns
the live progress of the active run (`{phase: building|registering, total,
registered}`, `progress: null` when no run is active); the daemon publishes
it for the status handler while a run is in flight and clears it on
completion, failure, or cancellation.

`repository_index_run` is long-running by design (like the CLI `index
repository` command) and runs per-connection, so concurrent runs are
last-write-wins with consistent snapshots.

## Realm federation

Realm federation is local Unix-socket transport between two instances. It does
not expose TCP/HTTP, share a provider database/blob store/index, or reuse a
provider daemon token.

The provider owns a current, durable realm-read grant keyed by a
domain-separated digest of the bearer credential. Before every provider
`federation_search` or `federation_evidence` request it reads the current
grant, verifies the provider and consumer realm, active state, access type, and
bounds, then composes the grant sensitivity ceiling with provider retrieval
policy before any candidate is retrieved or scored. Graph expansion is
disabled for federated search.

`federation_search` replies carry the provider realm, explicit graph
degradation, normal search provenance, and no more than the grant's result
bound. `federation_evidence` requires `search-and-open-evidence` access and
truncates the excerpt to the grant's evidence-byte bound. Missing, wrong, and
revoked credentials return a denial without provider data.

Only successful federated searches and evidence opens append an access record.
Grant issue and revocation are separate append-only events; revocation is read
from current state on the next provider request, so stale consumer bindings
cannot regain access after restart.

## Scope and provenance

Search uses the daemon's governed retrieval runtime, including ACL, trust,
sensitivity, quarantine, prompt-injection filtering, and source-selection
filters. Evidence requests use the core evidence-opening service, which
verifies source snapshots and hides records denied by retrieval policy.
Notebook metadata, source selections, and draft revisions are projections of
replayed authoritative state; mutation handlers submit domain inputs and wait
for their persistence barrier before acknowledging success. The daemon never
trusts a browser or agent to supply source identity, hashes, or citation
provenance.

The supported Rust client is `maestria_daemon::DaemonClient`:

```rust
let client = maestria_daemon::DaemonClient::from_instance(&layout)?;
let response = client
    .request(maestria_daemon::ClientOperation::Status)
    .await?;
```

`request` returns `Result<ClientResponse, DaemonRequestError>`. The error has
the typed `ClientErrorCode` above and a safe message. Callers should branch on
the code rather than parse the human-readable string.

This boundary keeps transport DTOs separate from domain entities while
preserving stable identifiers, search trace identity, source-selection digests,
evidence provenance, and validation-relevant task state.
## Retrieval status

`retrieval_status` (`ClientOperation::RetrievalStatus`, Studio `GET /api/retrieval`) is a
read-only instance-token operation consumed by Studio to render retrieval health.
It returns the current index generation and corpus snapshot, the deterministic
retrieval fingerprint, the live lane execution states, and the stored promotion
records backing them. Lane states are derived with the same fail-closed
derivations the live runtime applies at construction
(`runtime_construction::hybrid_policy`, `runtime_construction::learned_sparse_policy`);
the repository-code and visual lanes have no promotion path in runtime
construction today and always report `Shadow`. Any derivation or storage error
fails the whole read; no partial status is fabricated.

```json
{"type": "retrieval_status"}
```

Response (`RetrievalStatusResponse`) shape:

```json
{
  "index_generation": 3,
  "corpus_snapshot": 42,
  "fingerprint": "maestria-core:deterministic-v1",
  "lanes": {
    "hybrid_state": "Active",
    "hybrid_served_classes": ["DomainTerminology"],
    "hybrid_evaluation_id": "eval-123",
    "hybrid_evaluation_date": "2026-01-01",
    "hybrid_report_hash": "abc...",
    "learned_sparse_state": "Shadow",
    "learned_sparse_model": "naver/splade-cocondenser-ensembledistil",
    "dense_enabled": false,
    "dense_model": null,
    "repository_code_state": "Shadow",
    "visual_state": "Shadow"
  },
  "promotion_records": {
    "learned_sparse": null,
    "hybrid": {
      "evaluation_id": "...",
      "corpus_id": "...",
      "evaluation_date": "...",
      "report_hash": "...",
      "created_at": "..."
    }
  }
}
```

`hybrid_state` is `Shadow` | `Active`; `learned_sparse_state` is `Disabled` |
`Shadow` | `Active`; `dense_enabled` reflects the resolved dense generation and
`dense_model` the enabled manifest embedding model; promotion records are the
latest stored `RetrievalPromotionRecordWire` rows (`learned_sparse` / `hybrid`)
when present. See `crates/apps/maestria-daemon/src/api/search_services.rs:85`
(`retrieval_status`) and `crates/apps/maestria-studio/src/http/retrieval.rs`.


## Studio and ACP

Studio is an ACP v1 **client**. It does not implement or ship a model
provider, agent harness, filesystem callback, terminal callback, or MCP
server. Launch it after the daemon with:

```bash
maestria start -i <instance>
maestria studio -i <instance> --no-open
```

The CLI performs an authenticated `status` preflight. If the daemon is not
reachable it exits with exactly:
`daemon unavailable; start it with maestria start -i <instance>`.
Studio reads optional profiles only from
`<instance>/system/studio-agents.toml`; there is no current-working-directory
or CLI agent-config override. If that file is absent and `omp` is on `PATH`,
the in-memory built-in profile runs
`omp --no-tools --no-session acp`. Studio does not install, update,
authenticate, or configure that external command.

Each readiness probe and Ask starts a fresh ACP child/session. Initialization
negotiates protocol version 1, advertises no filesystem, terminal, elicitation,
boolean config-option, or MCP capability, and uses an instance-scoped agent
workdir. Only bounded text `agent_message_chunk` updates are accepted, and
the terminal stop reason must be `end_turn`. Permission requests are rejected
once when possible; unsupported callbacks cancel the turn. A profile timeout,
browser cancellation, or output overflow sends the ACP cancellation request,
closes stdin, waits briefly, then kills and reaps an uncooperative child.

The external agent must return one JSON object, with no surrounding prose or
fences:

```json
{
  "answer_markdown": "bounded Markdown",
  "citation_ids": [41],
  "draft_previews": []
}
```

Studio validates bounds, unknown fields, duplicate IDs, and citation
membership against the daemon context. Citation metadata is rebuilt from that
context; agent text is never persisted automatically. Draft previews remain
transient until an explicit typed `notebook_draft_save` mutation.

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


### `ModelAgentStatus` and `ModelAgentResolve`

`model_agent_status` reads the durable proposal, approval, harness, validation,
and memory-candidate projections for a `run_id`. It is read-only.

`model_agent_resolve` records an operator decision for the supplied
`approval_id` and resumes the durable model-agent continuation when approved.
The operation requires the authenticated daemon token and does not bypass
scope, effect-journal, validation, or stale-generation checks.
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

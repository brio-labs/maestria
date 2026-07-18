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

The response is one typed JSON envelope. Successful responses contain a
`response` object tagged with `status`, `search`, `evidence`, or `task`. Failed
requests contain an `error` string and never mutate domain state.

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

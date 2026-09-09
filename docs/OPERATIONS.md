# Operations Architecture

This document defines the durable contract for Maestria's runtime operations, state management, and recovery procedures.

## 1. Bounded Runtime Lifecycle

All tasks, queries, and background jobs operate within a bounded lifecycle:
*   **Initialization:** Resource allocation and context loading.
*   **Execution:** Active processing subject to hard timeouts and resource quotas.
*   **Termination:** Guaranteed cleanup, regardless of success, failure, or cancellation.

## 2. State and Recovery

*   **Journals:** Domain events and effect intents MUST be durably recorded before projections or non-idempotent adapter execution. Projections remain rebuildable.
*   **Recovery:** System restarts or crash recoveries replay the journal and reconcile persisted projections from the last valid checkpoint. In-flight non-idempotent effects pause unless explicitly resumed.
*   **Retries:** Idempotent operations support bounded automated retries with explicit backoff. Non-idempotent operations are not replayed after adapter execution begins without operator approval or a compensating action.

## 3. Execution Control

*   **Cancellation:** All long-running operations MUST be cancellable. Cancellation records a typed outcome, stops work at adapter-defined safe points, and releases resources; already committed external effects are not silently rolled back.
*   **Reproducibility:** Operations relying on stochastic models log model/index fingerprints, configuration, and random seeds where available. Reproducibility claims are bounded by the captured environment and corpus snapshot.

## 4. Data Evolution

*   **Migrations:** Schema changes to journals or persistent stores require explicit, forward-only migration scripts.
*   **Projection Rebuilds:** Read projections (e.g., search indexes, memory views) can be completely rebuilt from the immutable journal at any time.


See [ROADMAP.md](./ROADMAP.md) for the implementation schedule of these operational capabilities.

### Resource bounds

| Resource | Bound |
|---|---|
| Runtime input channel | Bounded capacity (1,024 intents); backpressure applied on saturation |
| Search limit | 1–100 per request |
| Search timeout | 120 seconds max |
| Parsing timeout | 60 seconds per file |
| Task workspace subdirectories | 5 (`context`, `evidence`, `drafts`, `validation`, `artifacts`) |
| Watcher polling interval | 1 second |
| Daemon client request size | 64 KiB max |
| Index generation limit | 32 per instance |
| Memory candidates per instance | 1,024 max |
| Concurrent harness effects | 4 (bounded by runtime worker pool) |

### Pause and resume

Continuous ingestion pauses when the daemon is stopped (`SIGINT`/`SIGTERM` or
service manager stop). The watcher persists its latest path/hash state to
`system/watcher-state.json` before shutdown; unchanged files are not reindexed
on restart.

Non-idempotent harness effects are paused during daemon recovery and are not
replayed without explicit operator approval. Idempotent operations (parsing,
indexing, validation) resume through the event-log recovery mechanism on the
next daemon start.

Task validation, approval resolution, and memory promotion are daemon-mediated
workflows. If the daemon stops during one of these operations, the operation
pauses and the operator restarts the daemon to resume. There is no hidden
background process — stopping the daemon guarantees no further I/O until the
next explicit start.

## 5. Continuous ingestion

The daemon starts a bounded polling observer only for manifest-approved
`read_root` paths. It skips excluded, hidden, and symlinked paths and accepts
the same document extensions as explicit indexing. A deterministic path/hash
state is persisted at `system/watcher-state.json`; unchanged files are not
submitted again, while changed files are sent through the existing bounded
domain-input channel. The observer stops with the daemon cancellation token and
persists its latest state before shutdown. A watcher-state persistence error
fails lifecycle shutdown and is returned to the daemon caller rather than being
logged and discarded.

The current observer uses a one-second polling interval and the runtime's
bounded channel for backpressure. To pause continuous ingestion, stop the
daemon (`Ctrl-C` or the service manager); restart it after changing the
manifest roots or exclusions. There is intentionally no hidden background
process or network watcher. Removed paths are retained in the watch-state
tombstone map for explicit operational review rather than being silently
forgotten.

## 6. Versioning Posture

Maestria is in continuous development with no external consumers: there are
no releases, no version ladder, and no milestone exit-evidence process.

- The workspace version is pinned at `0.0.0`; `main` is always the current
  build.
- Measurement evidence (benchmark reports) is recorded in
  `tests/contracts/benchmark_evidence_v1.json` and validated in CI, but it
  gates nothing and is tied to no milestone.
- Lint-exemption expiries in `scripts/philosophy_check` are calendar dates
  (`YYYY-MM-DD`), enforced by `philosophy-check` on every run.


## 7. Daemon-First Search Posture

Run one daemon per instance for interactive use (`maestria start -i <dir>`).
Daemon-served search is the fastest surface (measured 1.21 s versus 2.26 s
local on the benchmark instance during the #475 campaign), because it reuses
the daemon's warm retrieval runtime instead of assembling one per command.

The CLI is daemon-first and never requires the daemon:

- `search` submits to the instance daemon when its socket answers and prints
  `served=daemon`; otherwise it runs locally and prints `served=local`. The
  line makes benchmark and latency numbers attributable to a surface.
- Only an absent daemon (no token, no socket, or a refused connection)
  degrades to local execution. A daemon that is running but failing is an
  error, not a fallback trigger.
- While the daemon runs it owns the instance write lock: durable workflows
  (task validation, approval resolution, memory promotion, retrieval audit
  retirement) flow through the daemon surface, and direct mutation commands
  fail with the lock error instead of fighting over the instance.
- Lifecycle stays explicit (rule 28): the operator starts and stops the
  daemon. Tooling never spawns or stops one on demand; every surface that
  can degrade states so in its output rather than hiding the difference.

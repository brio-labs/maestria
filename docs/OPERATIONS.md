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
persists its latest state before shutdown.

The current observer uses a one-second polling interval and the runtime's
bounded channel for backpressure. To pause continuous ingestion, stop the
daemon (`Ctrl-C` or the service manager); restart it after changing the
manifest roots or exclusions. There is intentionally no hidden background
process or network watcher. Removed paths are retained in the watch-state
tombstone map for explicit operational review rather than being silently
forgotten.

## 6. Release Operations

Releases follow a staged workflow gated by milestone exit evidence and CI verification.

### 6.1 Milestone Exit Evidence

Every release milestone must carry a machine-readable exit-evidence block in its
description. The block is a fenced code block (` ```release-exit-evidence`) containing
a JSON payload that documents the release readiness stage.

The four sequential stages are:

1. **implementation-complete** — all issues closed, stage marker only.
2. **benchmark-complete** — benchmark measurements collected, may use synthetic/staged data.
3. **product-complete** — real benchmark data with passing quality/resource/security results.
4. **released** — publication complete with `post_release_work` tracking.

### 6.2 Exit-Evidence Tool

The `scripts/release_exit_evidence.py` script provides four subcommands:

| Subcommand | Purpose |
|---|---|
| `validate` | Validate a milestone description against contract rules |
| `generate` | Print a well-formed exit-evidence block to stdout |
| `reconcile` | Compare exit evidence against actual metrics |
| `validate-tracking` | Verify post-release follow-up completeness |

### 6.3 Release Workflow

1. Close all milestone issues and mark the milestone description with the
   appropriate `release_stage`.
2. Dispatch the [`release.yml`](../.github/workflows/release.yml) workflow with
   the target version, commit SHA, and milestone title.
3. The workflow preflight validates:
   - Milestone closure and all-issues-closed.
   - **Release exit evidence** via the `validate` subcommand.
   - CI run status for the target commit.
4. Verification runs the full [release contract](../scripts/release-contract.sh):
   tests, golden gate, lint, dependency checks, philosophy checks.
5. Build produces deterministic release archives with SHA-256 checksums.
6. Publish creates a tag and GitHub Release with generated release notes.

### 6.4 Post-Release Tracking

After release, `post_release_work` items must be tracked for completion.
Use `validate-tracking` to verify that all work items have valid statuses and
that completed items link to issue URLs.

### 6.5 Environment Consistency

Benchmark evidence SHOULD include environment fingerprints
(`benchmark.environment`) and artifact URLs (`benchmark.artifacts`) to ensure
reproducibility. The `reconcile` subcommand detects environment drift between
the evidence block and actual runner environments.

### 6.6 GoldenProfiles

Future release cycles MAY include a `profiles` section in the exit-evidence block
to track benchmark profile stages across releases. The supported profile stages
are `baseline`, `golden`, `shadow`, `promoted`, and `retired`.

### 6.7 Staged Workflows

For pre-production validation, milestones may use `data_fidelity: "staged"`
during `benchmark-complete`. Staged data cannot certify `product-complete`
or `released` stages; a real benchmark must pass before final publication.


## 6. Release Operations

Releases follow a staged workflow gated by milestone exit evidence and CI verification.

### 6.1 Milestone Exit Evidence

Every release milestone must carry a machine-readable exit-evidence block in its
description. The block is a fenced code block (` ```release-exit-evidence`) containing
a JSON payload that documents the release readiness stage.

The four sequential stages are:

1. **implementation-complete** — all issues closed, stage marker only.
2. **benchmark-complete** — benchmark measurements collected, may use synthetic/staged data.
3. **product-complete** — real benchmark data with passing quality/resource/security results.
4. **released** — publication complete with `post_release_work` tracking.

### 6.2 Exit-Evidence Tool

The `scripts/release_exit_evidence.py` script provides four subcommands:

| Subcommand | Purpose |
|---|---|
| `validate` | Validate a milestone description against contract rules |
| `generate` | Print a well-formed exit-evidence block to stdout |
| `reconcile` | Compare exit evidence against actual metrics |
| `validate-tracking` | Verify post-release follow-up completeness |

### 6.3 Release Workflow

1. Close all milestone issues and mark the milestone description with the
   appropriate `release_stage`.
2. Dispatch the [`release.yml`](../.github/workflows/release.yml) workflow with
   the target version, commit SHA, and milestone title.
3. The workflow preflight validates:
   - Milestone closure and all-issues-closed.
   - **Release exit evidence** via the `validate` subcommand.
   - CI run status for the target commit.
4. Verification runs the full [release contract](../scripts/release-contract.sh):
   tests, golden gate, lint, dependency checks, philosophy checks.
5. Build produces deterministic release archives with SHA-256 checksums.
6. Publish creates a tag and GitHub Release with generated release notes.

### 6.4 Post-Release Tracking

After release, `post_release_work` items must be tracked for completion.
Use `validate-tracking` to verify that all work items have valid statuses and
that completed items link to issue URLs.

### 6.5 Environment Consistency

Benchmark evidence SHOULD include environment fingerprints
(`benchmark.environment`) and artifact URLs (`benchmark.artifacts`) to ensure
reproducibility. The `reconcile` subcommand detects environment drift between
the evidence block and actual runner environments.

### 6.6 GoldenProfiles

Future release cycles MAY include a `profiles` section in the exit-evidence block
to track benchmark profile stages across releases. The supported profile stages
are `baseline`, `golden`, `shadow`, `promoted`, and `retired`.

### 6.7 Staged Workflows

For pre-production validation, milestones may use `data_fidelity: "staged"`
during `benchmark-complete`. Staged data cannot certify `product-complete`
or `released` stages; a real benchmark must pass before final publication.

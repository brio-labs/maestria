# Maestria

Maestria is a local-first, source-grounded second-brain runtime for AI agents.
It indexes your files, executes typed, query-adaptive searches, links evidence to
memory and tasks, and runs a restart-safe daemon for continuous operation — all
under explicit policy and validation gates.

## Install

Maestria targets Rust stable 1.95+. Build from source:

```bash
git clone https://github.com/brio-labs/maestria.git
cd maestria

# Build the CLI binary
cargo build --release -p maestria-cli
./target/release/maestria-cli --help
```

The GitHub release also provides a prebuilt Linux x86_64 archive containing
the user-facing `maestria` CLI and `maestriad` daemon binaries.


## Release development

The workspace version is defined once in `[workspace.package]` in the root
`Cargo.toml`. Package manifests inherit it, and runtime version constants derive
from `CARGO_PKG_VERSION`.

To prepare a version change:

```bash
python3 scripts/version.py set 0.6.2
python3 scripts/version.py check --expected 0.6.2
```

The `set` command validates the repository contract and refreshes `Cargo.lock`
through Cargo metadata. Release publication now requires a milestone exit-evidence report
in the milestone description as part of workflow preflight:

*   `implementation-complete`: all implementation issues are closed;
*   `benchmark-complete`: version-linked benchmark measurements are collected (synthetic is allowed but treated as provisional);
*   `product-complete`: real measurements include corpus/index/model fingerprints, quality/resource/security results, and degradations;
*   `released`: artifacts are published and follow-up work is explicitly listed in `post_release_work`.
    *   If synthetic or staged evidence remains pending, follow-up entries should target `maintenance/release` grouping when that grouping exists.

```release-exit-evidence
{
  "schema_version": 1,
  "release_stage": "product-complete",
  "benchmark": {
    "benchmark_date": "2026-07-19",
    "data_fidelity": "real",
    "fingerprints": {
      "corpus_snapshot": "corpus-v1",
      "index_generation": "idx-42",
      "model_fingerprint": "provider:rerank-v3"
    },
    "results": {
      "quality": {"status": "pass", "metric": "p50=0.74"},
      "resource": {"status": "pass", "p95_ms": 120},
      "security": {"status": "pass", "violations": 0}
    },
    "degradations": [
      {
        "area": "query_class",
        "status": "known",
        "note": "table evidence is incomplete on scanned PDFs"
      }
    ]
  },
  "post_release_work": []
}
```

The workflow must also enforce closed milestones and closed milestone issues.
## Quick start

```bash
# 1) Initialize an instance with approved read roots
maestria init -i .maestria-dev --read-root ~/Projects --read-root ~/Notes

# 2) Index a directory (recursive) or a single file
maestria index -i .maestria-dev -r ~/Projects/my-project
maestria index -i .maestria-dev ~/Notes/research.md

# 3) Search indexed chunks
maestria search -i .maestria-dev "source-grounded phrase"

# 4) Explain a durable search
maestria search explain -i .maestria-dev "source-grounded phrase"

# 5) Inspect evidence backing a search result
maestria open-evidence -i .maestria-dev --evidence-id 1
maestria open-evidence -i .maestria-dev --chunk-id 5

# 6) Inspect search/index/task observability
maestria search trace -i .maestria-dev 42
maestria index generations -i .maestria-dev
maestria evidence coverage -i .maestria-dev 7

# 7) Check instance health
maestria status -i .maestria-dev
maestria doctor -i .maestria-dev

# 8) Create and validate a task
maestria task start -i .maestria-dev "Review research notes"
maestria task add-evidence -i .maestria-dev 1 --evidence-id 1
maestria task request-validation -i .maestria-dev 1

# 9) Check task coverage and approve
maestria evidence coverage -i .maestria-dev 1
maestria approval list -i .maestria-dev

# 10) Propose and promote memory
maestria memory candidates -i .maestria-dev
maestria memory propose -i .maestria-dev -t "observation claim" -e 1,2 -c 700
maestria memory promote -i .maestria-dev -c 1 --approve

# 11) Start the daemon (or restart after changes)
maestria start -i .maestria-dev
# Stop with Ctrl-C; start again picks up where it left off
```

## Supported surfaces and capability status

### Daemon client

`maestria start -i <instance>` runs the local daemon. Its authenticated local
client boundary is newline-delimited JSON on
`<instance>/system/daemon.sock`; the token is stored in
`<instance>/system/daemon.token`.
`search`, `evidence`, `task`, and `model_agent_propose`. Requests without the
matching token are rejected, and read-only operations cannot mutate domain
state. `model_agent_propose` is a bounded, policy-gated proposal workflow: it
may search and request harness execution, but governance, validation, and
approval still control every side effect. See
[`docs/DAEMON-API.md`](./docs/DAEMON-API.md) for the request and response
envelopes and transport limits.

### Repository and document retrieval

Repository indexing and bounded context queries are supported:

```bash
maestria index -i .maestria-dev repository ~/Projects/my-project
maestria search -i .maestria-dev code symbol "SearchPlan"
maestria search -i .maestria-dev code context "RetrievalEngine" --depth 2 --nodes 32
```

PDF evidence preserves page/region provenance. Text/layout retrieval is the
stable route. Visual-provider retrieval is optional and remains shadowed unless
its frozen benchmark proves a quality and resource win; missing visual or OCR
providers degrade explicitly rather than fabricating text or coordinates.
Current-web queries require an enabled governed web adapter; without one they
use the bounded local fallback and expose the degradation in `search explain`.

### Tasks, validation, approvals, and memory

Task completion is validation-gated:

```bash
maestria task start -i .maestria-dev "check the repository"
maestria task request-validation -i .maestria-dev <task-id>
maestria evidence coverage -i .maestria-dev <task-id>
maestria approval list -i .maestria-dev
maestria memory candidates -i .maestria-dev
maestria memory propose -i .maestria-dev -t "claim" -e <evidence-id> -c 700
```

Memory proposals require evidence and remain candidates until the explicit
promotion policy is satisfied. Approval commands resolve governed requests;
they do not bypass scope or validation.

### Stable, degraded, and research-only routes

Stable local indexing, lexical search, evidence opening, daemon projections,
task validation, approvals, and evidence-backed memory candidates are shipped
with the current `0.6.1` binary. Repository/code and visual-document features
are implemented but remain release-visible capability surfaces with explicit
freshness/provider degradation. Advanced dense, learned-sparse,
late-interaction, graph/temporal, and multimodal promotions are
benchmark-gated: unavailable or unproven routes abstain or use a bounded
local fallback, and research candidates are not silently promoted.

The current workspace version and latest published release are independent
facts: `Cargo.toml` is the source for the next binary version, while
`v0.6.1` is the latest published release at the time of this documentation.
Release preflight requires version-linked exit evidence; closed issues alone do
not make a milestone released.

## Command reference

Every command accepts `-i, --instance-dir <PATH>` (default `.maestria-dev`).

### `init`

Create a local Maestria instance layout and manifest.

```
maestria init [-i <dir>] [--read-root <path>...]
```

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory (default `.maestria-dev`) |
| `--read-root` | Approved root path that may be indexed (repeatable) |

Omitting `--read-root` defaults to the instance directory itself.

### `index`

Index a file, files under a directory with `--recursive`, or list index
generations.

```
maestria index [-i <dir>] [-r] <path>
maestria index generations [-i <dir>]
```

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory |
| `-r, --recursive` | Recurse into subdirectories |

`index generations` reports generation lifecycle, serveability, corpus snapshot,
and representation fingerprint fields.

#### `index repository`

Build and persist exact Cargo metadata and Rust symbol records for a repository.

```
maestria index repository [-i <dir>] <path>
```

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory |
| `<path>` | Path to a repository root directory |

The command parses Cargo metadata and Rust source symbols into a persisted
code index that the `search code` commands query. The index is built under
manifest exclusion rules and must be inside an approved read root.

The observability names reserve `explain`, `trace`, `compare`, and
`generations` in their respective command positions. To use one as a direct
query or path, terminate option and subcommand parsing with `--`, for example
`maestria search -- trace` or `maestria index -- generations`.

### `search`

Search indexed local chunks or inspect durable search observability.

```
maestria search [-i <dir>] [-l <n>] <query>
maestria search explain [-i <dir>] [-l <n>] <query>
maestria search trace [-i <dir>] <trace_id>
maestria search compare [-i <dir>] <experiment_a> <experiment_b>
```

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory |
| `-l, --limit` | Max results (default 10) |


`search explain` executes a bounded search and prints its plan and trace.
`search trace` and `search compare` require durable, reproducible trace
payloads; missing or non-reproducible identifiers fail clearly.

#### `search code`

Query the persisted repository code index built by `index repository`. All
`search code` commands search the same persisted index and share the
`-i`/`--instance-dir` and `-l`/`--limit` flags.

```
maestria search code symbol <pattern>
maestria search code path <pattern>
maestria search code regex <pattern>
maestria search code context <pattern> [--depth <n>] [--nodes <n>] [--direction both|forward|reverse]
```

| Subcommand | Description |
|------------|-------------|
| `symbol` | Match repository symbols by name or qualified-name substring |
| `path` | Match repository symbols by source path substring |
| `regex` | Match repository symbols and paths with a regular expression |
| `context` | Traverse bounded repository relations from a symbol seed |

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory |
| `-l, --limit` | Max results (default 20) |
| `--depth` | Context traversal depth (default 2, context only) |
| `--nodes` | Max nodes in context response (default 64, context only) |
| `--direction` | Traversal direction: `both`, `forward`, or `reverse` (default `both`, context only) |

The code index is built from Cargo metadata and Rust source files. It is
validated against the instance manifest read scope before indexing and
queried with live freshness checks. Repository/code features are implemented
but are marked as provider-dependent and freshness-degraded until a frozen
benchmark proves a measured quality and resource win (see
[`docs/ROADMAP.md`](./docs/ROADMAP.md) Phase 4).


### `open-evidence`

Resolve typed source evidence without launching external programs.

```
maestria open-evidence [-i <dir>] (--evidence-id <n> | --chunk-id <n>)
```

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory |
| `--evidence-id` | Look up by evidence record id |
| `--chunk-id` | Look up by chunk id |

`--evidence-id` and `--chunk-id` are mutually exclusive; exactly one is required.

### `evidence`

Show evidence and validation coverage for a task.

```
maestria evidence coverage [-i <dir>] <task_id>
```


### `status`

Print local instance health facts: root path, database location, full-text
index directory, and event log count.

```
maestria status [-i <dir>]
```

### `doctor`

Check local storage, index, blob store, and parser wiring. Prints `ok` for
each component that opens successfully.

```
maestria doctor [-i <dir>]
```

### `start`

Start the Maestria daemon for the given instance.

```
maestria start [-i <dir>]
```

### `task`

Task workflow commands.

#### `task start`

Create a new persisted task.

```
maestria task start [-i <dir>] [-p low|normal|high] [--artifact-id <n>] <title>
```

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory |
| `-p, --priority` | Task priority: `low`, `normal` (default), `high` |
| `--artifact-id` | Link an existing artifact to the task |

#### `task show`

Show all tasks, or a single task by id.

```
maestria task show [-i <dir>] [<task-id>]
```

Omitting `<task-id>` lists every persisted task.

#### `task add-evidence`

Link an existing evidence record to a task.

```
maestria task add-evidence [-i <dir>] <task-id> --evidence-id <n>
```

#### `task request-validation`

Start validation for a task from a known task id.

```
maestria task request-validation [-i <dir>] <task-id>
```

#### `task complete`

Complete a validating task from a recorded validation report.

```
maestria task complete [-i <dir>] <task-id> --report-id <n>
```

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory |
| `--report-id` | Validation report id to confirm task completion |

Task completion is validation-gated: the domain requires a persisted,
task-matched, passing validation report and enforces warning/status consistency
before transitioning the task to complete state. Warning completion is permitted
only when the configured validation policy allows warnings.

### `memory`

Memory projection commands.

#### `memory candidates`

List persisted memory candidates.

```
maestria memory candidates [-i <dir>] [-l <n>]
```

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory |
| `-l, --limit` | Max candidates (default 20) |

#### `memory propose`

Propose a new memory candidate backed by evidence.

```
maestria memory propose [-i <dir>] -t <text> -e <id,...> -c <0..1000>
```

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory |
| `-t, --text` | Claim text |
| `-e, --evidence-id` | Comma-separated evidence ids (repeatable) |
| `-c, --confidence-milli` | Confidence in milli-units (0–1000) |

#### `memory promote`

Promote a memory candidate through governance-gated approval.

```
maestria memory promote [-i <dir>] -c <candidate-id> [--approve]
```

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory |
| `-c, --candidate-id` | Memory candidate id to promote |
| `--approve` | User approval for this promotion request |

Memory promotion requires a candidate that was previously proposed with
evidence backing. The `--approve` flag records user consent; without it the
promotion is submitted but not applied. Promoted memories are evidence-backed
and policy-gated (see [`docs/MEMORY.md`](./docs/MEMORY.md)).

### `approval`

Approval request management.

#### `approval list`

List pending approval requests.

```
maestria approval list [-i <dir>]
```

#### `approval resolve`

Resolve an approval request.

```
maestria approval resolve [-i <dir>] <id> (--approve | --deny)
```

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory |
| `--approve` | Approve the request |
| `--deny` | Deny the request |

Approval commands resolve governed requests; they do not bypass scope or
validation. Using both `--approve` and `--deny` together is rejected.

## Restart-safe policy-scoped workflow

The instance manifest records approved read roots and sensitive-path exclusions.
Indexing and search operate within these policy boundaries. The daemon
orchestrates recovery, reconciliation, and retry so that a restart picks up
where it left off without data loss or duplicate work.

## Architecture

| Crate | Layer | Description |
|-------|-------|-------------|
| `maestria-domain` | Kernel | Deterministic domain types, events, and transitions |
| `maestria-governance` | Governance | Decision gates and runtime adapter contracts |

## Invariants

- Domain and governance are side-effect free.
- All side effects are represented as typed intentions (effects).
- Policy and mechanism are separated by trait boundaries.
- Every change is validated through local checks and repository checks.
- Evidence is typed and source-grounded; raw strings are not evidence.
- Memory candidates point back to evidence. LLM output can propose; it cannot silently promote.

## Development

```bash
# Complete local gate: metadata, format, compile, lint, tests,
# release contract, docs, dependency, philosophy, and script checks.
bash scripts/verify-workspace.sh
```

Focused helpers remain available:

```bash
bash scripts/strict-clippy.sh
bash scripts/release-contract.sh
```

## Documentation map

- `docs/PHILOSOPHY.md` — repository doctrine
- `docs/SPECS.md` — invariant ledger
- [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) — system boundaries and ownership
- [`docs/SEARCH.md`](./docs/SEARCH.md) — typed retrieval contracts
- [`docs/MEMORY.md`](./docs/MEMORY.md) — source-backed memory lifecycle
- [`docs/SECURITY.md`](./docs/SECURITY.md) — scope, trust, taint, and secrets
- [`docs/OPERATIONS.md`](./docs/OPERATIONS.md) — runtime lifecycle and recovery
- [`docs/ROADMAP.md`](./docs/ROADMAP.md) — canonical implementation roadmap
- [`docs/RESEARCH.md`](./docs/RESEARCH.md) — dated non-normative evaluation candidates
- [`docs/architecture/`](./docs/architecture/) — architecture books
- `docs/first-pr-guide.md` — contributor onboarding

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md), [docs/first-pr-guide.md](./docs/first-pr-guide.md), and
[docs/PHILOSOPHY.md](./docs/PHILOSOPHY.md) for branch and review expectations.

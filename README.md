# Maestria

Maestria is a local-first, source-grounded second-brain runtime for AI agents.
It indexes your files, executes typed searches, links evidence to memory and
tasks, and runs a daemon for continuous operation — all under a restart-safe,
policy-scoped workflow.

## Install

Maestria targets Rust stable 1.95+. Build from source:

```bash
git clone https://github.com/wkral/maestria.git
cd maestria

# Build the CLI binary
cargo build --release -p maestria-cli

# The binary is at target/release/maestria
./target/release/maestria --help
```

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

# 8) Start the daemon
maestria start -i .maestria-dev
```

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
# Formatting
cargo fmt --all -- --check

# Linting
cargo clippy --workspace --all-targets --all-features -- -D warnings -D clippy::too_many_lines -D clippy::cognitive_complexity -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic -D clippy::disallowed_methods

# Tests
cargo test --workspace --all-targets --all-features

# Documentation
cargo doc --workspace --no-deps --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features

# Architecture guardrails
python3 scripts/philosophy-check.py
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

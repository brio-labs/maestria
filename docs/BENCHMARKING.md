# Benchmarking

How to produce performance numbers for Sillage that survive review. A
number produced by this document's method is a measurement; anything else
is an estimate, and estimates are labeled as such or left out.

## Rules

1. **Measure wall clock of the release binary.** Build with `cargo build
   --release`, capture real elapsed time. Debug builds and extrapolations
   are not benchmarks.
2. **Fresh instance per run.** `init` a new instance directory for each
   measured run. This resets instance state; it does not empty the OS page
   cache, caches held outside that instance, an already-running daemon, a
   model/provider cache, or a remote service. Name process, index, model,
   application, OS, and remote cache states separately. A new instance is
   not an OS-cold run.
3. **Identical workload, controlled change.** Use the same corpus, flags,
   workload, and declared machine conditions. Change one factor for an
   ablation; for end-to-end system comparisons, record the full configurations
   and give each the same admissible tuning budget.
4. **Baseline first is diagnostic, not final order.** Measure the unmodified
   build on the same day and instance shape as a diagnostic baseline. For a
   final A/B comparison, alternate or randomize paired blocks; do not let a
   fixed baseline-first order stand in for a counterbalanced comparison.
5. **Repeat and report spread, failures, and denominators.** Short commands:
   `hyperfine --warmup 1`. Minute-scale timing runs need repeated observations;
   a few long runs do not support precise p95/p99 claims. Report independent
   information needs/clusters separately from timing repetitions: repeating
   one need improves its timing description, not the independent quality
   sample size. Keep failures, refusals, timeouts, and cancellations visible
   in the attempt and task denominators; never average only successful runs.
6. **An interrupted run poisons the next measurement.** Killing an index
   run mid-batch leaves recovery work in the durable log; the next open pays
   crash repair before anything else. A search benchmark on that instance
   measures repair, not search. Keep the interrupted attempt and its status;
   discard or separately label the contaminated subsequent measurement, not
   the historical failure record.
7. **Derive ratios on real data before generalizing.** A synthetic corpus
   gives stable A/B deltas, not absolute transferability. Index a slice
   of real repositories — heterogeneous mixes, sizes, governance
   refusals — and confirm synthetic per-file cost sits inside the
   real-data envelope before publishing ratios.
8. **Micro-bench the changed path.** An embedding-transport change gets a
   sequential-vs-batch latency curve; a tantivy-threading change gets
   writer-throughput timing. The end-to-end number confirms; the
   path-local number explains.


## Evidence formats and validation

The existing v1 manifest remains a historical milestone ledger with its
original validation rules and frozen-source resolution. It is not a generic
run record, and adding run fields does not retroactively validate or revise
its reports. Its report bytes are checked only when explicitly requested:

```sh
python scripts/benchmark_evidence.py \
  --manifest tests/contracts/benchmark_evidence_v1.json \
  --report-root /path/to/benchmark-reports
```

Schema v2 is one immutable run/attempt record. Validate its structure with:

```sh
python scripts/benchmark_evidence.py --manifest /archive/run.json
```

This reports structural validity only; artifacts are unverified and cannot
support a confirmatory claim until a local artifact root is explicitly
provided:

```sh
python scripts/benchmark_evidence.py \
  --manifest /archive/run.json \
  --artifact-root /archive/artifacts
```

V2 artifact paths must be relative local paths beneath that root. The
validator does not fetch network URIs; it rejects traversal and symlink
escapes, and checks listed bytes against each artifact's declared digest,
size, media type, and access classification. An inaccessible private or
controlled artifact remains unavailable/unverified and makes a confirmatory
claim ineligible. A structurally valid manifest alone is never reproduction.
The validator reads records; it does not create a run, fabricate receipts, or
rewrite an original record.

An attempted run records a start receipt before execution and a linked
terminal receipt afterward. A start without a terminal receipt is
`unfinished`, not `complete`; terminal outcomes remain distinct as
`complete`, `failed`, `timeout`, `cancelled`, or `invalid`. Preserve every
attempt, including crashes and failed setup. Corrections receive a new run
identity and link the original; never overwrite the original record. For each
measurement, `value`, `unit`, `status`, `reason`, `method`, and denominator
are explicit. A real measured zero is allowed; `unavailable`, `not_run`,
`not_applicable`, and `invalid` require `value: null`, never a zero
placeholder. Denominators distinguish independent needs/clusters from timing
repetitions.

Each v2 run records its evaluation attempts and separately sourced campaign
consumption for preparation, tuning, and setup. The ledger preserves every
first evaluation outcome and retry; later attempts never replace the frozen
first-attempt observation. Campaign compute and cost totals include both
collections, including failed, cancelled, and timed-out work, and must match
the ledger exactly while remaining within the declared budget. Preparation
wall time is the sum of preparation activity records, not a value copied from
the manifest.

Confirmatory eligibility is narrower than schema validity. The manifest must
bind a frozen protocol, source bytes, baseline and candidate configuration
bytes, execution identity, receipts, access/retention policy, budgets, and
reviewer provenance. Every start receipt binds the verified protocol digest,
freeze ID, and frozen time, and must be recorded after the freeze; terminal
receipts link to and follow their starts. All six process/index/model/
application/OS/remote cache boundaries must have explicit known states, and
RAM must be a positive integer. Product-telemetry-origin runs require
affirmative telemetry opt-in; an explicitly started experiment may remain
eligible without product telemetry opt-in.

Randomized final ordering and these v2 trace requirements govern new
protocols. They do not alter the recorded order, claims, artifacts, or
qualification status of historical v1 evidence.

### Content-bound v2 eligibility

Artifact digests establish byte identity, not evidence meaning. A manifest
cannot qualify a run by repeating protocol bindings, measurement names,
budgets, or observations in its own fields. Role-tagged evidence includes the
frozen protocol, corpus/splits/qrels and case-list bytes, execution record,
baseline and candidate configuration records, declared budget, attempt
ledger, campaign-consumption records, access record, observations, and
start/terminal receipts. Supplemental artifacts do not replace these roles.

The protocol freezes source and system digests, execution identity, required
measurement definitions, the ordered first-attempt roster, and a positive
`max_concurrent_attempts` limit. Each planned slot also freezes its first
`run_id` and `attempt_id`; the ledger's first row for that slot must retain
those identities. Each system configuration digest must match its verified
configuration artifact bytes. Case-list bytes establish independent case and
cluster counts; source bytes must match their frozen digests. A start receipt
binds the protocol artifact digest, freeze ID, and frozen timestamp, and is
recorded after that timestamp. First-attempt start receipts must follow frozen
roster order; each retry must start strictly after its predecessor's terminal
receipt. Evaluation and campaign-consumption overlap is bounded by the frozen
concurrency limit: overlap is rejected at one and allowed only up to the
explicitly declared maximum. Each attempt occupies a concurrency slot from
its start receipt until its linked terminal receipt. An unfinished attempt
continues occupying its slot because no terminal receipt releases it.
Finished attempts have terminal receipts recorded after their starts;
unfinished attempts retain an absent terminal outcome.

The ledger preserves every planned first evaluation outcome before retries.
Separate preparation, tuning, and setup activity records carry their own
attempt IDs, receipts, compute, cost, and wall time; failed setup attempts and
their retries remain visible. Campaign compute and cost totals equal the sum
of evaluation and campaign-consumption rows and remain within the declared
budget. Preparation-wall-time limits use the sum of preparation activity
records.

Confirmatory retrieval comparisons require baseline and candidate nDCG@10,
plus the candidate-minus-baseline paired mean difference for matched
case/repetition outcomes. Both systems also report fast-path and
first-document latency p95 (nearest-rank aggregation), plus peak memory and
disk, against maxima frozen per measure in the protocol. Every observation
artifact binds the ledger digest and covers each required system's first
attempt slots, including failed first outcomes; a successful retry cannot
substitute for them. Values and denominators must match the frozen aggregation
method. Access records bind consent, access, and retention policy; matching
records do not waive affirmative opt-in for product-telemetry-origin runs.

A correction requires a verified original-record artifact whose path, bytes,
immutable digest, run ID, and attempt ID match the correction reference.
Manifest and evidence versions are integer-typed; booleans are not version
numbers. These content checks apply only to v2 confirmatory claims and do not
alter the historical v1 ledger or its frozen-source/report resolution.


## Harness

Corpora and instances live outside the repository (default
`~/sillage-perf/`, with `corpus*/` inputs and `inst-*/` instances):

```sh
cargo build --release -p sillage-cli
CLI=target/release/sillage-cli
$CLI init  -i inst-a --read-root corpus >/dev/null
{ time $CLI index -i inst-a -r corpus --yes ; } 2>&1 | grep real
{ time $CLI search -i inst-a "query text"     ; } 2>&1 | grep real
```

Read the run's own telemetry as well: the `status: files N/M rate=…
bytes=…` progress line reports steady-state throughput, and the summary
line reports indexed/unchanged/skipped/failed counts. Cross-check the
counts against the corpus before trusting the time — a run that silently
skipped half the corpus is faster and meaningless.

Search has distinct variants; name the one measured. Read-only open
loads index generations only; durable open loads full kernel state;
daemon-served goes over the UDS protocol. They differ by seconds at
scale.

## Profiling

Profiling answers "where does the time go", never "is it slow". Measure
first, profile second.

- `perf record -F 199 -g -o /tmp/x.data -- <cmd>`; read results with
  `perf report`, which resolves symbols where `perf script` output can
  fail.
- `samply` works in interactive mode; saved-only captures lack symbols.
- `heaptrack` records fine but launching its GUI blocks pipelines;
  analyze with `heaptrack_print file.zst`.
- A sparse profile with no dominant frame means the process was waiting,
  not computing: find what it waits on (sidecar, locks, drain paths)
  before touching CPU paths.

## Measured traps

Every item below was estimated one way and measured another:

- **Ratio-math speedups.** Windowed CLI submission was estimated ~20%
  faster from wait-loop arithmetic; measurement showed −1.6% because the
  runtime, not the CLI loop, dominated. Estimates set hypotheses; runs
  set claims.
- **Concurrency tuning is a curve.** Vector-lane permits 2 → 8 improved
  dense ingest 40%; permits 16 regressed past the 2 baseline (ONNX
  oversubscription). Measure both directions before shipping a knob.
- **Pipeline serialization hides gains.** A second tantivy writer thread
  measured 19.2 s vs 18.6 s because the upstream stage serialized.
  Locate the serialization point before adding parallelism.

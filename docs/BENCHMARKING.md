# Benchmarking

How to produce performance numbers for Maestria that survive review. A
number produced by this document's method is a measurement; anything else
is an estimate, and estimates are labeled as such or left out.

## Rules

1. **Measure wall clock of the release binary.** Build with `cargo build
   --release`, capture real elapsed time. Debug builds and extrapolations
   are not benchmarks.
2. **Fresh instance per run.** `init` a new instance directory for every
   measured run. A reused instance carries a residual event log, warm
   projections, and prior index generations that change the result.
3. **Identical inputs, one variable.** Same corpus, same flags, same
   machine state as the baseline. Change exactly one thing between
   baseline and candidate.
4. **Baseline first, same day.** Measure the unmodified build before the
   change, on the same corpus and instance shape — not from an old note.
5. **Repeat and report spread.** Short commands: `hyperfine --warmup 1`.
   Minute-scale runs: run twice; disagreeing beyond noise means a third
   run and reporting the range.
6. **An interrupted run poisons the next measurement.** Killing an index
   run mid-batch leaves recovery work in the durable log; the next open
   pays crash repair before anything else. A search benchmark on such an
   instance measures repair, not search. Index to completion, or discard
   measurements from the poisoned instance.
7. **Derive ratios on real data before generalizing.** A synthetic corpus
   gives stable A/B deltas, not absolute transferability. Index a slice
   of real repositories — heterogeneous mixes, sizes, governance
   refusals — and confirm synthetic per-file cost sits inside the
   real-data envelope before publishing ratios.
8. **Micro-bench the changed path.** An embedding-transport change gets a
   sequential-vs-batch latency curve; a tantivy-threading change gets
   writer-throughput timing. The end-to-end number confirms; the
   path-local number explains.

## Harness

Corpora and instances live outside the repository (default
`~/maestria-perf/`, with `corpus*/` inputs and `inst-*/` instances):

```sh
cargo build --release -p maestria-cli
CLI=target/release/maestria-cli
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

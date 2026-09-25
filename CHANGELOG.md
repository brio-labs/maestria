# Changelog

Rolling development log. Maestria has no releases: the workspace version is
pinned at `0.0.0` and `main` is always the current build.

## [Unreleased]

Maestria activates the dense embedding lane for the lexical/hybrid search
route on benchmark evidence, adds live indexing metrics to the CLI, and
introduces the repository index selection layer: a whitelist-first choice
surface (CLI, daemon API, studio, and web) that scopes repository code
intelligence to a reviewed set of directories.

### Added
- Linux-first resident Maestria Launcher (Tauri 2, React 19, GTK3/WebKitGTK 4.1):
  isolated application catalog, host commands, calculator, native selected-file
  open/copy actions, keyboard-first Preferences, X11 shortcut setup, Wayland
  portal and compositor-owned activation fallback, explicit `--activate`/`--quit`,
  Debian/AppImage packaging, and opt-in content-free renderer-ready timing.
  The separate native Slint launcher offers non-modal first-run shortcut setup;
  deferral persists across Debian/AppImage X11 launches. Packaged X11 AT-SPI
  smoke exercised calculation-to-clipboard, desktop-entry launch, successful
  shortcut setup/restoration, Caps/Num-lock activation, and rejection of a
  conflicting grab without changing the saved shortcut. An isolated packaged
  Weston Wayland run with a fake seat verified first-run AT-SPI deferral/focus
  and resident reactivation/quit. Ubuntu 24.04's Weston 13 lacks a fake seat;
  the locally exercised no-seat path verifies deferral persistence, not focus.
  The final Debian/AppImage were rebuilt against Ubuntu 24.04 after a
  host-built Debian required unavailable `GLIBC_2.43`; the package verifier
  now rejects executables above Ubuntu 24.04's `glibc` 2.39 ABI. Apt
  installation without the search service and visible X11 startup passed in
  a disposable Ubuntu container; its first-run AT-SPI offer check timed out.
  The exact Ubuntu-built Debian/AppImage payloads passed X11 and nested
  Weston UI smoke on the host, not Ubuntu container accessibility acceptance.
  Hosted Ubuntu CI, live portal grant/denial, and chooser acceptance remain
  open. The launcher neither starts the daemon nor indexes files. Optional
  separate-process passage search is now configured in schema-1 launcher.toml;
  an isolated X11 Slint run displayed a highlighted Markdown citation and
  full document view, copied freshly reopened evidence, refused the same
  action after the source changed, and showed a later app query without the
  old passage. Nested Weston Wayland showed cited detail, copied passage and
  citation after authenticated reopen through a private X11 clipboard fallback,
  and refused Open source after the file changed. The actual native clipboard
  helper copied through standard Wayland in isolated Weston with `DISPLAY`
  absent and no data-control protocol; an independent `wl-paste` received the
  text. This does not prove the complete pure-Wayland Slint action or live
  portal decision, and whole-path latency/native accessibility coverage
  remain open.
  The current source-built Slint open action now displays the exact reopened
  excerpt and citation before sending a verified PDF page hint to the default
  handler (`#page=N`); a disposable X11 daemon/UI run exercised genuine Markdown
  and page-1 PDF actions, including a private viewer that received the fragment.
  External viewers may ignore that hint; no pure-Wayland action or rebuilt
  Ubuntu package is certified by this source-built smoke.
- Sillage extensions: a versioned manifest and TypeScript SDK/example, sealed
  directory/ZIP bundles with explicit permission-diff consent, host-rendered
  list/detail/form views, and a separately installable QuickJS worker invoked
  only inside bubblewrap. The broker scopes search to a separate owner-issued
  external realm, accepts only fresh cited v2 previews, and bounds desktop
  actions, HTTPS, storage, bytes, time, and worker cancellation. Store mutations
  use a cross-process lock and descriptor-relative nofollow removal.
  In an isolated native Slint X11 session, the installed Greetings ZIP showed
  an authorized `greeting.txt` excerpt and copied through the real clipboard;
  denying a notification-expanding 1.0.1 update preserved the approved 1.0.0
  grants. A deliberate worker exception surfaced without crashing the launcher,
  closing the panel interrupted an infinite-loop worker, revocation blocked
  commands despite a still-active separate provider grant, and removal
  preserved or deleted private data according to the selected choice. The
  manager remained usable without the worker sibling binary. An Ubuntu 24.04-
  built standalone worker Debian apt-installed alongside `bubblewrap` in a
  disposable Ubuntu container; the Ubuntu-built executable ran there and
  returned a typed invalid-arguments error. Hosted CI, full pure-Wayland
  extension UI actions, live portal grants/denials, active-indexing latency,
  and combined-product release acceptance remain open.
  The launcher package verifier now rejects package relationships with the
  optional search and worker; a previously built Ubuntu Debian/AppImage passed
  the updated verifier in a disposable Ubuntu image. The launcher-only native
  smoke now requires both optional packages absent and restricts all launcher
  interactions to the installed `/usr/bin` executable; this updated installed
  smoke remains to be run on a clean rebuilt Ubuntu package.
- Graceful shutdown drain: with `drain_effects_on_shutdown`, the runtime
  keeps servicing domain inputs while in-flight effects finish, so an
  effect completing after cancellation still delivers (and persists) its
  completion input. Exit is quiescence-driven — no queued batches, no
  running effects — bounded by a `shutdown_drain_grace` ceiling, so quiet
  sessions shut down as promptly as before.
- Retrieval audit retention (ADR-0009): `retire-retrieval-events` emits a
  governed append-only marker through a live daemon or a local session;
  the journal loader stops decoding the state-free audit family below it,
  `status` reports the boundary, and retired trace lookups answer
  explicitly. No deletions.
- Daemon-first search posture (#483): `search` prints `served=daemon|local`
  so numbers are attributable, and `docs/OPERATIONS.md` documents the
  recommended per-instance daemon posture with the explicit, never
  auto-started lifecycle.
- A versioned `sillage.search` v1 Unix-socket search, status, and evidence API
  accepts bounded, revocable per-consumer grants without sharing the provider
  instance token. The search-only `maestria-search` CLI and separate Debian
  artifact provide explicit root initialization, read-only daemon start,
  owner roots/grants, and typed external search/evidence/status/indexing-status.
  A separate local process found an indexed phrase, opened cited evidence,
  and denied an ungranted realm without a launcher or configured model.
  Search-only grants can receive byte-bounded, typed cited previews without
  evidence-open permission; edited sources and revoked grants do not release
  stale previews. The separate Debian apt-installed in an Ubuntu 24.04 container:
  its v2 interactive operation returned a cited Markdown passage, DOCX
  paragraph citations reopened with exact snapshot checks, text-bearing PDF
  pages reopened with an authorized typed PDF path; previews exposed no PDF
  path, and an image-only PDF counted as OCR-needed. Durable restart,
  changed-source search and direct evidence-open denial, and grant
  denial/revocation also passed locally. On 10,000 indexed Markdown files,
  197/200 uninstrumented warm interactive requests succeeded (successful
  separate-process CLI p50 68.05 ms, p95 78.42 ms, p99 110.7 ms); three
  timed out at the unchanged 100 ms daemon deadline. After approving a further
  700 files, the first six of 30 calls timed out with indexing pending,
  followed by 24 successes while indexing reached zero pending; this does not
  prove active-indexing latency. Direct-path lookup now avoids scanning the
  active-source map for ordinary file/DOCX previews while retaining normalized
  fallback, fresh hashes, scope checks and audit persistence. A later isolated
  200-call warm probe on the fully indexed retained corpus returned 199
  successes (successful CLI p95 51.20 ms) and one 100 ms timeout; with daemon
  tests running concurrently it returned 196 successes and four timeouts.
  These different workloads do not prove a causal speedup or deadline pass.
  Hosted CI and whole-path native latency remain open.
  A same-process A→B→A edit again indexed and served the original
  content-addressed artifact without restart or regrant. Replaying the
  predecessor B tombstone after restored A now leaves A active and non-stale;
  fresh source reopen succeeded after restarting the rebuilt provider. The
  final search-only Debian passed a fresh Ubuntu 24.04 install and the
  installed-process package smoke after this correction.
- Bounded `sillage.search` v2 interactive queries also return typed
  filename/path-only results for fresh indexed text sources, distinct from
  cited passages; PDF paths remain suppressed. An independent consumer found
  an approved filename missing from its body, while an out-of-root name,
  deleted source, removed root, and revoked grant returned no path. The native
  Slint File/Path row copied only after a fresh authorized search; deleting
  its source left the clipboard unchanged and displayed a denial. On a
  settled 521-file private corpus, first cold searches timed out four times
  and three times after restart at the unchanged 100 ms daemon deadline.
  The daemon now prepares the interactive source snapshot before socket
  readiness, reuses one event replay and source projection for its engine,
  prunes filename mismatches before source approval checks, and persists
  watcher state as compatible compact JSON. The rebuilt search-only daemon
  returned 30/30 late-filename matches in independent-process CLI calls after
  restart (first 46.3 ms; median 33.915 ms; maximum 69.0 ms). A first query
  after a source edit still timed out, and deletion caused two timeouts
  before no-path responses. Neither active-indexing nor whole-native-path
  latency acceptance is claimed.
- For lexical-only interactive snapshots, SQLite now scans just the indexed
  parser-start, captured-document and stale-source events, while repository
  code-security snapshots retain full history. On the same private 521-file
  corpus, a sampled startup scan of all event families took 141 ms; the
  narrowed scan took 18–41 ms in later startup/update observations. The
  first changed-body query after the watcher settled still hit the unchanged
  100 ms daemon deadline: in one trace snapshot construction reached 43 ms,
  root filtering 50 ms and lexical retrieval 101 ms before evidence reopening
  or audit. The warm retry returned the changed cited passage. Active-indexing
  and native whole-path latency remain unaccepted.
- Interactive consumer retrieval now builds approved/grant-scoped candidate
  filters in one pass and keys its warm cache by the exact manifest and root
  grant. A live two-root A→B→A passage regression passed. A local CLI smoke
  over 521 short Markdown files returned a changed cited passage in 66.7 ms after
  indexing settled and denied a deleted source in 48.7 ms; the initial cold
  request still timed out at the unchanged 100 ms daemon deadline. This does
  not certify active-indexing latency or native p99.
- Provider read grants now freeze one or more exact approved roots per newly
  issued consumer in schema v18 and versioned grant events. Owner CLI accepts
  repeated `--read-root`; omitted flags freeze all roots approved at issuance.
  Legacy NULL-scoped grants still follow all currently approved roots and
  must be revoked/reissued before approving a newly sensitive root. Root
  scopes are capped at 64 roots and 8 KiB of path bytes and fail closed if
  stored metadata is malformed. A live two-sibling-root daemon regression
  denied the other root's passages, filename paths, direct evidence and
  consumer inventory before source I/O. Independent source-built CLI
  processes issued distinct A-only/B-only grants, retrieved each permitted
  cited passage and A's filename-only path, suppressed the other root in
  ordinary and interactive search, and reported only one A-root indexed file.
  A direct A→B evidence reopen returned typed `SourceNotSelected`. A live
  two-root integration also indexed a one-page PDF under B; B opened its
  actual page-1 excerpt and source path, while A could neither search its
  passage nor open its authentic evidence ID (`SourceNotSelected`). Both
  grant scopes persisted after daemon restart; removing A's provider-approved
  root dropped A's passage and inventory to zero while B stayed readable.
  A fresh Ubuntu 24.04 `target/search-packages-current/` Debian artifact
  passed package verification and apt-installed Markdown/DOCX/PDF, stale-edit,
  restart, grant-revocation and two-root isolation smoke. After approving B
  under an existing A-only grant, A's indexing inventory stayed at one root,
  B's own consumer searched and reopened its passage, and A could not
  search B's source or reopen B's authentic evidence (`SourceNotSelected`);
  both grants survived restart. Fresh Ubuntu launcher Debian and AppImage
  payloads in `target/launcher-packages-current/` passed payload and ABI
  verification. Combined apt installation and a time-bounded native smoke
  passed in disposable Ubuntu 24.04 after enabling `org.a11y.Status.IsEnabled`
  on the test's private session bus. AccessKit had left the visible Slint
  window's AT-SPI tree inactive while this property was false; the earlier
  `window=False`, `buttons=[]` was test setup, not a product failure. The
  installed Debian and fresh AppImage exposed the first-run offer, deferral,
  Preferences, resident reactivation, keyboard and clipboard actions; X11
  shortcut setup survived restart and rejected conflicting grabs. Nested
  Weston confirmed Wayland startup, offer/deferral and quit; the host run
  used a fake seat, while the Ubuntu run did not. Neither checked Wayland
  passage actions or live portal grants.
  A prior local container invocation hung because `xvfb-run` ran as PID 1;
  later runs kept it as a child under a hard timeout. Older default output
  directories predate these root-scope and source-event changes.
- `docs/OPERATIONS.md` now documents opt-in launcher-only, search-only,
  combined, and optional extension-worker Ubuntu Debian installation,
  approved roots, scoped credentials, explicit daemon start/stop, removal,
  and data choices. Disposable Ubuntu apt installs resolved all four choices
  without daemon autostart; search-owner grant and SIGINT shutdown passed.
  Refreshed Ubuntu-built launcher/search Debian artifacts then passed combined
  apt installation: the installed search CLI found a granted filename-only
  result, denied it after deletion, and removing search left the launcher and
  user-owned instance. On private host X11, executables byte-identical to
  final Ubuntu-built Debian payloads rendered Slint File/Path and highlighted
  body-only Markdown citation rows; Copy Path and Copy Passage wrote
  independently authorized content to the private clipboard, and the deleted
  path displayed an explicit denial without overwriting it. The launcher
  Debian now declares `wl-clipboard` for pure Wayland. The older default
  AppImage predates the filename/path and clipboard changes; the refreshed
  AppImage passed the native package smoke above. An upgraded desktop-portal
  grant, Wayland passage actions and combined search release certification
  remain unverified.
- Hybrid dense-lane activation: a real-instance benchmark
  (`maestria_hybrid_evaluation`, manual) measures lexical vs hybrid recall on
  six query classes with RAPL energy telemetry and writes a promotion record
  bound to the report hash; `VocabularyExpansion` and `DomainTerminology` are
  served by the dense lane (`hybrid_state=Active`).
- Live `index` metrics: per-file progress, embedding counts from the vector
  projection WAL, throughput, and a final summary with MiB/min and embeddings.
- `index repository` — whitelist-first repository code indexing with
  `--include`/`--all`/`--yes` selection, classified candidate tree, and
  per-directory policies (`max_file_bytes`, `skip_generated`, `skip_minified`);
  identity, delta, records, and freshness are selection-scoped.
- Repository index selection surface: daemon operations
  (`repository_index_candidates`/`selection_get`/`selection_save`/`run`/
  `status`/`children`/`files`/`progress`), studio HTTP proxy routes, and the
  web workspace with lazy candidate-tree browsing and run progress polling.
- Hot-path performance: startup reconciliation is watermark-guarded so
  unchanged instances skip all projection reconciles (search start drops from
  seconds to milliseconds), the tantivy writer commits once per session
  instead of per operation (read paths flush for read-your-writes, `Drop`
  preserves durability), CLI state polling reuses one SQLite connection
  across poll iterations, and `KernelState` collections are Arc
  copy-on-write so staged inputs clone pointer handles instead of deep
  entity/event copies (~17x fewer allocations on an index workload), and
  domain inputs now apply in place under the runtime write lock with
  replay-to-persisted-prefix repair on failure, removing the remaining
  per-input candidate clone (index user CPU drops ~78% on a 1,200-file
  corpus; the cost no longer grows with the event-log size).
- Fixed per-invocation costs: stored-event payload validation now runs only
  when the database schema changes (fresh database or migration) instead of
  decoding the whole event log as JSON on every store open (single-command
  latency on a 60k-event instance drops ~15%), and `Scope` stores its root
  and pattern lists behind shared slices so cloning an effect execution
  context no longer deep-copies governance configuration per effect.
- SQLite write connections use WAL `synchronous=NORMAL`: commits stop
  fsyncing per transaction, which removes an fsync stall per persisted
  batch during indexing (1,200-file corpus drops from 6.6 minutes to under
  2 minutes). Application crashes remain safe — only OS-level power loss can
  lose the most recent commits, and startup recovery re-drives any pending
  work from the durable event log.
- Repository queries reuse parsed SQLite statements (`prepare_cached`), so
  per-hit authorization lookups no longer re-plan SQL on every call.
- Read-only search startup skips the full event-log replay: the retrieval
  runtime consumes only the index-generation registry, which rebuilds from
  the self-contained generation events (13 of ~60k), and evidence rendering
  reads the durable evidence store the engine already authorizes against
  (lock-contended searches drop from 3.9 s to 3.2 s on a 60k-event
  instance). Event-log replay also stops deep-cloning every envelope.
- Slim `chunk_registered` events: the payload no longer duplicates the
  chunk text inside every representation (`raw`/`retrieval` mirror it),
  storing representation kinds plus a stable digest instead. Legacy rows
  with inline contents keep replaying and recompute their digest on read.
  On a 1,200-file corpus this cuts event-log payload bytes by ~62%
  (111 MB → 42 MB), shrinks the database ~26%, speeds indexing ~25%, and
  drops durable-search startup from 3.9 s to 2.3 s on a fresh log.
- `Chunk` carries that `representations_digest`, so restart recovery
  compares registrations by identity instead of requiring both sides to
  hold full representation contents.
- CLI `search` dispatches to a live instance daemon first (`daemon.sock` +
  instance token) and renders from the same durable evidence projection,
  falling back to local execution only when the socket accepts no
  connection or `--task-id` scoping requires full local state. Daemon-served
  searches drop from 2.3 s to 1.2 s on a 60k-event instance because the
  warm daemon runtime replaces the per-invocation state load; a new
  `daemon_unavailable` error code distinguishes "nothing serving" from
  real failures so the fallback never masks errors.
- Vector-lane concurrency raised from two to eight permits: dense-lane
  embedding runs now use the host core count instead of serializing behind
  a pair of in-flight requests (dense indexing of a 250-file slice drops
  from 134.6 s to 81.8 s; sixteen permits over-subscribe ONNX inference
  and regress). The local embedding sidecar also accepts array `input`
  batches, matching the OpenAI embeddings contract.
- `privacy_exclusions` defaults covering machine-state directories, wired
  into index-selection scanning and blocked patterns.
- `IndexGenerationRegistry` with lifecycle transitions and retired-generation
  rollback, plus `MonotonicInstant` saturating arithmetic.
- Vector projection WAL mode and `embedding_row_count()` for concurrent
  readers.

### Changed
- The current `maestria-launcher` binary now uses Slint instead of the
  Tauri/React renderer. The resident window, query focus, Preferences,
  discoverable Slint About attribution, light/dark theme, X11/Wayland
  activation, and Debian/AppImage packaging were exercised locally; both
  packages passed native X11 keyboard and AT-SPI smoke. The executable,
  Debian package ID, desktop/portal ID, and `launcher.toml` remain stable.
  Passage results and full launcher parity are not yet implemented; legacy
  frontend files remain pending the parity cleanup.
- Schema v17 expires all previously issued realm read grants at fixed Unix
  second 1. Existing event IDs and payloads stay unchanged and replay is
  deterministic; old credentials cannot read any passages or evidence. On the
  provider, list grants, revoke each expired digest, then explicitly issue a
  new time-bounded grant for that consumer. Revoked grants remain revoked.
- Dependency batch: workspace pins advanced to thiserror 2, ureq 3,
  rusqlite 0.40, getrandom 0.4, tokio 1.53, and tokio-util 0.7.19 in one
  migration. The ureq 3 transport keeps the no-redirect policy and the
  public-DNS-only resolver for web evidence; `getrandom::getrandom`
  call sites move to `getrandom::fill`. No behavior changes.
- Search execution budgets: default candidate/work ceilings raised
  (30 000 / 30 000 000) so the dense lane scans the whole projection instead
  of a truncated `chunk_id`-ordered prefix — the previous top-K was only the
  best of the first scanned rows.
- Promotion gate: `source_redundancy` bound relaxed from +20% to +100%
  relative, reflecting the structurally larger candidate set of a fused
  two-lane result.
- Read-only search runtimes (CLI search while the daemon holds the instance
  lock) now derive the hybrid policy from the persisted promotion record
  instead of hardcoding shadow.
- `content_hash` identity unified on the chunk text so projection
  reconciliation skips re-embedding of unchanged chunks.
- The code-intel source registration window constant moved to the daemon
  (`repository_source_registration.rs`); `code_intel_sources.rs` removed.
- The repository code-intel walk now applies `PrivacyExclusions::default()`
  (machine-state and credential-shaped paths) so `index repository` enforces
  the same privacy boundary as the generic whitelist-first indexer.

### Fixed
- Approved-root watcher status now waits for a durable parser receipt instead
  of treating channel submission as indexing completion. Edits, deletions,
  and revoked roots deny stale search/evidence; reapproving unchanged content
  restores its original content-addressed version across restart. External
  `indexing-status` reports aggregate progress without source paths.
- A retained `RuntimeHandle` now holds only a weak reference to the
  runtime-owned search executor, so an old handle cannot keep Tantivy's
  writer alive after shutdown. Generation fingerprinting reads a current
  index without claiming a writer; a lifecycle regression covers a
  dirty-watermark restart with the old handle retained. The separately
  reported intermittent lock on a first fresh CI fixture remains unproven.
- Launcher boundary fixes: stale searches retain Preferences scope; replaced
  catalog results cannot dispatch actions; file open/copy checks the accepted
  generation through native dispatch; invalid saved shortcuts retain their file
  with a warning; reset clears the binding before persisting defaults; worker
  startup errors abort launch; and stale timing requests cannot erase newer
  measurements. Copy feedback no longer hides catalog failures.
- Instance write-lock liveness records the holder's process start ticks:
  after a crash with pid reuse, a stale lock was treated as held by a live
  process, wedging the instance (tests hit this as flaky approval/memory
  workflow failures under parallel load; #489).
- Dense lane `BudgetExhausted` only when an exhausted lane produced no
  candidates — a lane with results was misreported and could fail the search
  trace/plan match.
- Full collection of all documents no longer marked `truncated` (one-past-end
  candidate limits in the lexical chunk/card adapters).
- Repository index runs and candidate scans canonicalize the root before
  selection containment, matching the CLI path (a non-canonical root could
  silently drop Cargo targets).
- Web selection tree: a file selected individually stays checkable instead of
  being covered by its own selection.
- Ingestion throughput: the runtime full-text effect now batches every
  pending chunk of an artifact into one projection commit (one `IndexFullText`
  effect serialized per artifact), replacing the per-chunk Tantivy commit;
  a 2,000-file home-scale root drains in minutes instead of stalling for
  hours under segment flush/fsync churn.
- Daemon memory on large roots: with the pipeline draining, RSS plateaus once
  ingestion completes instead of growing monotonically while pending
  artifacts accumulate (previously the process could be killed externally).
- The learned-sparse benchmark instrumentation record binds `report_hash` to
  the frozen corpus content instead of an arbitrary format string, so the
  record is verifiably tied to its evaluation evidence.
- The learned-sparse benchmark's `peak_ram_bytes` is now the per-route
  process RSS delta around the route's run (search plus lifecycle
  operations), not the process-wide `VmHWM` high-water mark, which the first
  measured route dominated.
- Studio Index workspaces pre-fill the instance root path from the daemon
  bootstrap status instead of requiring the path to be typed by hand (the
  sanitized display name never looked like an absolute path).

## [0.6.1] — 2026-07-20

Maestria v0.6.1 adds code intelligence indexing/search, memory promotion,
task completion, and approval resolve commands to the CLI. It also completes
the documentation surface with all supported commands now explicitly listed
in the README and establishes a documentation consistency gate.

### Added

- `index repository` — build and persist Cargo metadata and Rust symbol index.
- `search code` with `symbol`, `path`, `regex`, and `context` subcommands.
- `task complete` — validation-gated completion from a recorded report.
- `memory promote` — governance-gated memory promotion with user approval.
- `approval list` and `approval resolve` — manage pending approval requests.
- Documentation consistency checker (`scripts/doc-consistency-check.py`) that
  derives the command tree from `cli_types.rs` and verifies README coverage.
- Version-to-phase mapping and explicit stable/shadowed/provider-dependent/
  research-only status markers in the roadmap.

### Changed

- README now documents the full CLI surface including all subcommands, daemon
  boundary, code/repository retrieval, task lifecycle, approval management,
  and memory promotion.
- Quick start covers the complete init-to-restart workflow.
- The verify-workspace gate now includes the documentation consistency check.


## [0.6.0] — 2026-07-18

Maestria v0.6 is the **Query-Adaptive Search** release. It turns the retrieval
baseline into a bounded, inspectable search workflow that reports both evidence
and uncertainty instead of treating every query as a fixed top-k lookup.

### Added

- Policy-validated search plans with deterministic intent classification,
  capability checks, scopes, budgets, modalities, freshness, and stop
  conditions.
- Deterministic query rewriting and stage-aware decomposition with rewrite
  accounting in the durable search trace.
- Bounded iterative retrieval with explicit no-evidence, incomplete, conflict,
  stale, warning, and answerable outcomes.
- Evidence packs with claim coverage, source independence, conflict and
  counterevidence metadata, missing evidence, compression lineage, and
  reproducibility fingerprints.
- Governed web discovery/evidence separation: discovery results remain
  candidate URLs until fetched, snapshotted, provenance-checked, and policy
  checked.
- Retrieval and security validators for plan validity, provenance, coverage,
  conflicts, freshness, citation alignment, ACL/trust/quarantine, and
  regression budgets.
- Operator observability commands:
  `search explain`, `search trace`, `search compare`, `index generations`, and
  `evidence coverage`.
- Restart-safe runtime recovery, projection reconciliation, ignore-aware
  directory traversal, task/evidence linking, and source-backed memory
  candidates from the completed earlier milestones.
- Frozen deterministic golden-query fixtures and release-gate execution in CI.

### Changed

- All workspace packages and public component version constants now report
  `0.6.0`.
- The release workflow gates publication on the closed v0.6 milestone, passing
  CI, exhaustive verification, a locked release build, checksum validation, and
  CLI/daemon smoke tests.
- Hyphenated identifier-like queries are treated as exact lookups so common
  filenames, slugs, and secret-like paths do not get misclassified as an
  unsupported natural-language constraint query.

### Verification

The release gate exercises formatting, Clippy policy, workspace tests,
documentation, dependency checks, philosophy checks, the frozen golden gate,
locked release builds, deterministic Linux artifacts, SHA-256 checksums, and
extracted CLI/daemon `--help` smoke tests.

[0.6.1]: https://github.com/brio-labs/maestria/releases/tag/v0.6.1
[0.6.0]: https://github.com/brio-labs/maestria/releases/tag/v0.6.0
- Windowed index submission in the CLI (windows of eight) with the per-file
  pipeline split into `commands/index_batch.rs`; real-corpus validation
  recorded (synthetic per-file cost sits inside the real-data envelope).
- `docs/BENCHMARKING.md`: measurement protocol for performance work —
  fresh-instance timing discipline, real-corpus envelope validation,
  profiling guidance, and measured estimation traps.
- Embedding provider batching groundwork: `EmbeddingProvider::embed_batch`
  with a true array implementation in the loopback HTTP provider.
- CI path-gating: benchmark-evidence ledger changes validate the manifest
  only (new `evidence-validate` job) instead of running the full retrieval
  bench shards; pure-Python `scripts/` edits no longer trigger the whole
  Rust CI matrix (#480).
- `docs/adr/ADR-0008`: per-artifact vector effects proposal — replaces
  per-chunk `IndexVector` effects with one `IndexArtifactVectors` effect
  per artifact generation, with pending-vector tracking in `KernelState`.
- Per-artifact vector effects (ADR-0008): one `IndexArtifactVectors` effect
  per artifact replaces per-chunk `IndexVector`, consuming `embed_batch`
  with a single projection upsert; dense slice 81.8 s -> 40.6 s (2.0x).
- Vector indexing completion delivery at shutdown no longer fails the
  effect: the projection is durable, so a delivery that races teardown is
  a logged, bounded bookkeeping deferral (#486).

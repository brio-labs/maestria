# Operations Architecture

This document defines the durable contract for Sillage's runtime operations, state management, and recovery procedures.

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


See [ROADMAP.md](./ROADMAP.md) for the product milestones and their required
operational evidence.

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

### Generated target storage maintenance

Before large producer or consumer operations, use
`scripts/generated-storage-maintenance.py guard` to measure generated `target`
storage. The invocation is read-only by default. Replace these example paths
with the existing target directory and its current useful root:

```bash
python3 scripts/generated-storage-maintenance.py guard \
  --target-root /absolute/path/to/target \
  --current-root /absolute/path/to/target/current-useful-root
```

The defaults are a **15% high watermark**, **10% low watermark**, and **10%
minimum filesystem-available capacity**.

For a complete inventory, the watermark uses the entire target's apparent-byte
metric: regular-file `st_size` plus non-followed symlink pathname lengths
across current, pinned, leased, and unknown roots. Known FIFO, socket,
character-device, and block-device entries are reported separately as
`retained_special_entry_count`; their unspecified `st_size` is not treated as
apparent bytes, and they remain preserved. This metric is not physical or
safely reclaimable space. Physical headroom uses filesystem `statvfs`
available bytes. Dry-runs report unchanged pre-maintenance capacity. After a
triggered apply, post-maintenance capacity is measured after the guard
`fsync`s the target directory; apparent bytes removed do not promise an equal
physical-capacity increase.

No pruning occurs without explicit `--apply`. An unsupported inode type, mount
boundary, or metadata-access failure in preflight blocks candidate discovery
with exit 2 and `measurement-blocked`. Incomplete target totals are null; a
separately labeled `known_apparent_bytes_lower_bound` is provided only when
available, and `watermark_ok` is null rather than a pass. If inventory becomes
incomplete after apply, completed prune/modify history and the measured
post-`fsync` `statvfs` capacity remain in the report, but the result is still
`measurement-blocked`. A complete, triggered dry-run exits 2 with
`dry-run-maintenance-required`. A triggered apply exits 0 only when the guard's
capacity and watermark gates pass; otherwise it exits 2 with the blocking
status and preserved or skipped roots. Unknown roots are not inferred as
disposable, so unresolved pressure remains blocked.

When ordinary permissions deny metadata traversal of an owned subtree, the
guard remains measurement-blocked. An operator may choose an already
owner-authorized UID/GID-mapped, read-only namespace for metadata-only
inventory if that existing mapping grants traversal. The guard does not
create mappings, elevate privileges, change modes or ownership, or infer
pruning approval from this route; without it, `EACCES` remains blocking.

Declare additional current roots with repeatable `--pinned-root` and live build
inputs with `--lease-root`. `--cargo-target-root` authorizes only the
`incremental` and `.fingerprint` cache children; `deps` and `build` outputs
remain in place. An explicitly retired root via `--retired-root` is required
to authorize whole-profile Cargo `deps`/`build` pruning. Any relevant Cargo
locks are acquired and revalidated before each destructive candidate. Current,
pinned, and leased roots; source files; Cargo lock files; and small canonical
first-outcome records remain preserved. Linux `/proc/self/fdinfo` mount-ID
evidence is required. If mount identity or boundaries cannot be verified, the
guard fails closed rather than pruning.

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

The observer polls every second. At most eight detections await a durable
`ParserStarted` receipt; the bounded runtime channel applies additional
backpressure. Deferred sources remain pending and are retried after a receipt,
not treated as indexed. Receipt probes skip deferred observations so a large
root cannot starve the small enqueued set; newly confirmed slots can be reused
in that scan. Ingestion publishes full-text changes before emitting successful
completion feedback. To pause, stop the daemon (`Ctrl-C` or the service manager);
restart it after changing manifest roots or exclusions. There is no hidden
background process or network watcher. Removed paths remain in the watch-state
tombstone map for explicit operational review.

## 6. Versioning Posture

Sillage is in continuous development with no external release promise.

- The development workspace version is `0.0.1`; `main` is always the current
  build.
- Each product milestone in [ROADMAP.md](./ROADMAP.md) requires its own
  observable exit evidence. Historical retrieval reports do not satisfy a
  product-milestone gate.
- Measurement evidence (benchmark reports) remains recorded in
  `tests/contracts/benchmark_evidence_v1.json`. Its original source inputs are
  archived under `tests/frozen-corpus-snapshots/benchmark-evidence-v1/` at
  recorded source commit `193a44d4bb2800a3ee19f44728362aa5ccfdc8cc`; CI
  validates the archived bytes against the manifest hashes only. Frozen
  `source_paths` are provenance, not paths into active Sillage code. Historical
  results do not qualify the renamed source or complete product-milestone gates.
- Frozen `RealMaestriaTask` metadata decodes to the distinct
  `HistoricalWorkTask` provenance category and keeps its original serialized
  label. Conversion to a benchmark does not relabel it as `RealSillageTask`.
  Historical work cannot qualify current Sillage sparse or Hybrid serving;
  this metadata reader is not an old-name runtime/configuration fallback.
  Doctrine checks exclude archived Rust compilation units from active-code
  rules while general source and secret scans continue to inspect the archive.
- The first canonical 60-need FR/EN real-model cohort completed execution but
  failed qualification: applicable lexical heads were preserved in 27/28
  cases, and the exact-path target regressed from rank 2 to 7. Judgments were
  LLM-assisted and independently source-reviewed, not human-certified gold.
  These cases/controls are consumed; serving remains Shadow, with no persisted
  promotion. Correcting aggregate-result lane admission does not constitute
  a new model qualification or establish an exact-path repair.
- Full native passage citation/excerpt values are accessibility labels;
  `Full citation` and `Full returned passage excerpt` are semantic descriptions.
  Observers must read actual names/text rather than expect those descriptions
  as constant names. The zero-query cached-row before/after control proves
  content exposure, not retrieval, Return navigation, or installed latency.
- A distinct canonical synthetic-query discriminator recorded one durable
  search and one evidence reopen, with full detail values and the copied notice
  in its complete final AT-SPI tree and viewed screenshot. Its observer report
  builder and failure handler failed; the first outcome remains failed and
  closed. No clipboard/timing acceptance, historical attribution, or installed
  qualification is inferred from that partial evidence.
- The first canonical CI run `37083359067` failed test, nextest, Studio, and
  search-package jobs; its first logs/outcomes remain immutable. Activation
  mechanics fixtures are explicitly counterfactual, not fabricated benchmark
  qualification. The authenticated cold CI-image build identified Dioxus's
  second CSS compiler, a different Wasm LTO profile, and absent `rust-src`.
  `web/studio.css` avoids Dioxus Tailwind autodetection; the pnpm CSS compiler,
  ThinLTO/16-unit Wasm profile, and pinned source component are explicit inputs.
  A cold image build with the actual pinned component reproduced the four
  committed bundle hashes; no bundle drift validator was weakened.
- Regular Search retains its partitioned candidate capacity before fusion and
  enforces the final result ceiling afterward. InteractiveSearch retains its
  prior result window and 100-ms deadline. An independent cross-lane consensus
  regression failed before and passed after; this does not fix unavailable
  searches or establish historical navigation/latency causality.
- Separate exact-package and repaired-source diagnostics returned a partial
  DOCX match for an unquoted multiword query. A subsequent owner-only runtime
  trace showed both lexical lanes succeeded: Markdown ranked first in chunks,
  but DOCX won RRF with card and chunk support. Shell argument quoting is not
  Tantivy phrase syntax: an exact literal CLI query must include double quotes,
  for example `'"the literal passage"'`. All main-phrase package-smoke callers
  use that syntax across initial, restart, source-change and denial gates.
  Fresh source Search passed; separate InteractiveSearch and evidence reopen
  passed at 64.42 ms CLI round-trip. The owner trace does not qualify authorization;
  none of these synthetic source results qualifies exact-installed behavior,
  ordinary-query FR/EN relevance, failed Return, or the original unavailable
  searches. The original CI response was not retained; it cannot be recovered
  or causally attributed from these observations.
- Lint-exemption expiries in `scripts/philosophy_check` are calendar dates
  (`YYYY-MM-DD`), enforced by `philosophy-check` on every run.

### Integration branch CI

`push` and `pull_request` cover `main` and `dev/sillage`; the pull-request
branch list selects the target (base) branch. The `v*` tag trigger and manual
`workflow_dispatch` remain enabled.

The PR filter uses `dorny/paths-filter@v4.0.1`, whose default `some` predicate
ORs each list pattern; `!scripts/*.py` is not an ordered exclusion. In the
existing `rust` list, `scripts/*` matches top-level Python scripts, while the
negated pattern matches other changed paths.

| Event or PR change | Existing gate behavior |
|---|---|
| Any non-empty PR diff, including docs-only or test-only changes | `rust=true`; the full existing matrix runs, and `philosophy` runs as well. |
| Push to `main`/`dev/sillage`, `v*` tag, or `workflow_dispatch` | No PR path filter; all event-applicable gates run. |

The Rust matrix retains formatting, Rust/Studio tests, documentation, audit,
benchmark, and package jobs, including launcher-native, launcher-portal,
search-package, extension-worker, combined-extension, version-upgrade, and
installed-native-benchmark. The path list explicitly names root `src/`,
`crates/`, `web/`, `launcher/`, `extension-sdk/`, workflow/actions, and
package-smoke helpers. `philosophy` also matches its documentation/checker
paths. `changes` always runs on PRs; `commit-branch-check` runs on every PR
regardless of path. CI has no standalone TypeScript extension-SDK build/test.


### First outcomes and source binding

CI run `37525357443` remains failed and unchanged: pull-request head
`af94c764652526c66f1cca199a6cf879b069d048` was `dev/sillage` targeting `main`.
Run metadata reports 20 successful jobs and four failures. Retained job logs
locate the first failure points without establishing deeper causes:

- job `112485122655`, `installed-native-benchmark`: four interaction observations failed
  or timed out; the reference-hardware gate also reported effective memory below
  16 GiB and SSD storage unestablished.
- job `112485122744`, `launcher-portal-package`: the checker did not observe the
  expected consent request in its private portal monitor.
- job `112485122745`, `version-upgrade-package`: the current Sillage search check
  did not finish indexing one approved Markdown file within 120 seconds.
- job `112485122812`, `combined-extension-package`: after the
  extension-permission revoke step, the expected revocation notice was not
  exposed to the accessibility checker.

These are observed gate outcomes, not diagnoses of the underlying runtime
causes; this CI-01 change does not rerun or rewrite them. The historical #550
report is bound by PR #560 metadata to head
`eb53d34cc91cfa719065efbc55ecadb55f402606`, merged as
`42981b92f9e2617689906380a3dad9a13936910a`. The #551 comment reports head
`552543961ff39414571ef28637775591c2ebf92e`, but PR #561 metadata gives actual
head `55254396e8aada226d2c3adbcdf109f9df72cfb5`; retain that mismatch rather
than attributing the report to the CI head. `main` remains Shadow, PR #516
remains draft, and qualification remains false.

### Source-only worktree preparation

For isolated repository work, set `OWNED_WORKTREE` to a fresh, empty,
task-owned location and `BASE_SHA` to the exact base commit. Register without
checkout or hooks, then materialize source only in that newly registered,
empty worktree:

```bash
/usr/bin/git -c core.hooksPath=/dev/null worktree add --no-checkout "$OWNED_WORKTREE" "$BASE_SHA"
/usr/bin/git -C "$OWNED_WORKTREE" read-tree --reset -u HEAD
```

Run `read-tree --reset -u HEAD` only in that fresh owned worktree, never over
an existing/shared tree or an unowned path. Do not use destructive Git cleanup
or reset operations on shared, existing, or uncertain paths. Do not copy build
caches, per-agent `target/` directories, evidence, frozen/consumed roots, or
binaries into source worktrees. A prior interrupted standard creation left an
unregistered target copy; its cause is unestablished. Allocated-extent totals
without exclusive extents do not establish unique physical growth or a Git
fault. Treat an unexpected copy as an owner-reviewed incident, not an automatic
cleanup target. Shared `target/` storage can include evidence, frozen/consumed
roots, binaries, and shared data; do not assume it is reclaimable cache.
Preserve those inputs. Parent-owned integration verification runs centrally
only after free-space and Btrfs metadata safety checks. Workers do not run
builds, tests, lint, formatters, or gates, and do not create per-agent caches or
targets.

### Desktop launcher lifecycle

The launcher package installs a desktop entry for the user to invoke; installing
it does not launch the application or enable login autostart. The entry runs
`sillage-launcher --activate`. Shortcut setup is user-initiated. A fresh launcher
does not start document search. Explicit Enable in Document search Preferences
starts a separate launcher-owned read-only service; saved explicit managed
consent may resume it on later launcher starts. Disable or launcher shutdown
stops that owned process; hiding the resident window does not. No model starts.
An operator-managed external daemon remains under its operator's control.


## 7. Daemon-First Search Posture

Run one daemon per instance for interactive use (`sillage start -i <dir>`).
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
- Operator-managed CLI lifecycle stays explicit (rule 28): the operator starts
  and stops that daemon. Queries never spawn or stop one on demand; every surface
  that can degrade states so in its output rather than hiding the difference.
  The separately owned launcher workflow below requires explicit Enable or saved
  explicit managed consent, not an implicit start triggered by a query.

## 8. Opt-in Linux package onboarding

Debian artifacts built from this source revision use version `0.0.1` and
architecture `amd64`; CI builds them on Ubuntu 24.04. From the repository root,
install the launcher, headless search service, or both with `apt` so Ubuntu
resolves the declared runtime dependencies:

```bash
# Apps only: no search daemon or worker is installed.
sudo apt install ./target/launcher-packages/sillage-launcher_0.0.1_amd64.deb

# Search only: no launcher or worker is installed.
sudo apt install ./target/search-packages/sillage-search_0.0.1_amd64.deb

# Combined: explicitly select both components.
sudo apt install \
  ./target/launcher-packages/sillage-launcher_0.0.1_amd64.deb \
  ./target/search-packages/sillage-search_0.0.1_amd64.deb
```

### Daily-driver utilities and login behavior

**Utilities** provides keyboard selection (Up/Down, Return), creation (Ctrl+N)
and saving (Ctrl+S). Quicklinks accept one `{query}` in an HTTP(S) URL outside its
authority and encode the argument before native dispatch. Snippets expand only
literal `{query}` and copy on explicit request; other markers remain literal.
Save edited templates before expanding them. These utilities do not start
document search or a model, monitor typing, or intercept global snippet keys.

Quicklinks and snippets are private plaintext in
`${XDG_CONFIG_HOME:-$HOME/.config}/io.github.briolabs.Sillage.Launcher/utilities.toml`.
Back up that file only if you intend to retain its text. Writes are bounded,
no-follow and atomically published, with ownership/content revalidation;
foreign or symlink inputs are preserved rather than replaced intentionally.

Clipboard history requires **Save clipboard**, retains at most 100 entries of
64 KiB each in memory, expires them after one monotonic hour and clears on
restart. Select/copy, per-entry Delete and Clear are explicit. Closing or
switching panels clears copied editor/model buffers; it does not clear the
unexpired in-memory collection. Native capture has a 500-ms deadline and a
64-KiB output bound. On Linux its helper caps its own soft/hard address-space
limits at 256 MiB without raising tighter inherited limits. No secret detection,
encrypted history or automatic capture is promised.

General Preferences offers opt-in login autostart. Only the exact owned
`${XDG_CONFIG_HOME:-$HOME/.config}/autostart/io.github.briolabs.Sillage.Launcher.desktop`
entry is managed, using `sillage-launcher --background`. Startup stays resident
without mapping a window; a duplicate background invocation does not activate
it. Explicit activation shows and focuses the existing instance. Autostart and
singleton cleanup revalidate ownership and socket identity and preserve foreign
inputs, but POSIX checks followed by unlink are not an atomic unlink-if-inode
guarantee against a final same-UID path race.

For offline calculations, enter arithmetic or `<number> <unit> to <unit>` and
copy with Return. Length, mass, temperature, duration and digital storage are
supported; this is not a live currency or model-backed conversion service.

The window has a 760×760 preferred size and a 640×540 logical-pixel minimum.
Preferences use a scrolling viewport with pinned Save/Done and document
Enable/Disable actions rather than a fixed-size window.
Utility rows are bounded rather than stretched to fill the results viewport.
Native control palettes initialize from the restored theme as well as later
theme changes, so saved dark mode keeps checkbox captions readable on restart.

The current daily-driver acceptance recipe targets a private Ubuntu 24.04 X11
desktop and ordinary installed executables. Earlier Wayland/AppImage results
are bound to their own sources and packages; they do not qualify this newer
daily-driver payload. Physical monitor changes, hardware suspend/resume,
current-source Wayland activation/chooser behavior and full screen-reader support
remain unverified. Same-version payload replacement is not a version upgrade;
the separate canonical package-version proof below closes that lifecycle item
without an upstream workspace-version bump or retrieval/release qualification.

The fresh installed `daily-driver-installed-thirty-fourth` scope completed
61 consumer observations in 94 seconds against producer-tenth and
archive-validation-eleventh. It used the real GTK folder chooser, browsed
directory rows instead of accepting an autocompleted child, and kept search
off until explicit Enable. After source/folder dispatch, the fixture uses the
ordinary `sillage-launcher --activate` command before continuing: launching an
external handler need not leave the launcher mapped.

It covered initial Indexing/Ready, two independent freshly cited passages,
saved-consent restart, exact source/path/folder actions and long-path copying,
Disable, actual 640×540 and scale-two surfaces, utility/extension persistence,
owned autostart and APT reinstall/removal. Cleanup reaped all owned children
and proved desktop/namespace absence within five seconds without force-kill.
The 58 original XWDs have paired decoded PNGs and individual visual notes.

Evidence destination:
`~/.local/share/sillage-release-evidence/daily-driver-packaged-result-first/`.
Run its `verify-daily-driver-result.py` with that directory as its sole argument
for read-only inventory/hash, source/package binding, consumer-invariant,
capture and cleanup checks. It does not execute products or access the network,
SQLite, private profiles, credentials or raw audits. Closed first failures,
unused inputs and the older first-run seal are not replayed or overwritten.

Fresh `version-changing-upgrade-installed-third` installed all three original
producer-eighth canonical packages at `0.0.1`, upgraded to the real changed
producer-tenth payloads packaged as `0.0.1+daily-driver.1`, allowed an explicit
APT downgrade to the old packages, re-upgraded and removed the suite normally.
The full Debian package version changes; the repository workspace stays at
`0.0.1`, and neither frozen producer nor earlier seal was changed.

Each package phase checked observed dpkg identity/version/architecture and exact
`/usr/bin` payload hashes/mode `0755`. Saved preference, utility-definition and
extension-grant/data bytes matched before launching each transitioned version.
Real UI consumption checked saved dark preferences, quicklink dispatch, exact
snippet copying and permission-backed Text Tools copying after upgrade,
rollback and re-upgrade. Rollback kept those bytes even after the old UI consumed
them; newer launches additionally measured compact utility rows. The search
payload remained identical; the worker hash changed without an inferred behavior
change. The two actual product-source changes are utility sizing/scroll stride
and initial native palette assignment, not solely package metadata.

The runtime passed 105 observations in 165 seconds, with 89 original XWD/PNG
pairs and seven bounded normal owned shutdowns. Ordinary package removal retained
the remaining user data. Both earlier harness first failures and unused document
inputs remain preserved; no closed scope was replayed.

Separate immutable supplement:
`~/.local/share/sillage-release-evidence/version-upgrade-packaged-result-first/`.
Run `verify-version-upgrade-result.py` with that directory as its sole argument.
It checks the six actual Debian archives, real source/payload differences,
installed upgrade/rollback/re-upgrade consumers, capture inventory and owned
cleanup without executing products or reading private profiles/SQLite/network.
Current-source Wayland, physical monitor/suspend, full screen-reader,
reduced-motion and remote HTTP credential-provider qualifications remain open;
Shadow, draft PR #516 and qualification false are unchanged.


### Installed local provider grant evidence

The historical #550 report records 32 native observations on canonical Debian
`0.0.1+provider.1`, bound by PR #560 to the exact source head noted above. Only
`sillage-search` was rebuilt; launcher and worker payloads were the previously
inspected native binaries. The report covers read-only owner review, explicit
issuance, SDK search/copy, exact-root and consumer denials, natural expiry,
reviewed renewal, and owner revocation. The local search-provider grant was
distinct from extension permissions; #551 separately observed a local grant
remain active across HTTP-grant revocation. These are historical test-time
states, not evidence of current grant status. The launcher grant remained
independent and active during those observations; no current-source UI
acceptance is inferred.

The approved policy was search-only, Internal sensitivity, one exact root,
two results and 4096 evidence bytes. Local ingestion retains its existing
Internal classification; no synthetic document was reclassified to bypass it.

The historical comment reports a 28-file seal passed read-only verification;
that reference is not evidence of present archive availability. All ten earlier
preparation/runtime first outcomes remain preserved and no closed scope was
replayed. Extension disable/revoke does not qualify launcher Document Search
Disable or independently administered-provider shutdown.
Preset extension form entries can expose an AT-SPI Entry role without a Text
interface, including after focus. Native keyboard editing still works. The
installed harness reads actual selected input through Ctrl+A/C and restores
the exact private clipboard; it never substitutes expected text or an empty
default. This is not full screen-reader qualification. Dynamic extension
status, notices, permissions, details, removal identities and pending-worker
messages expose their actual content as accessibility labels with stable
semantic descriptions.

### Host-owned HTTP credentials and Secret Service vault binding

The historical #551 report records 45 native observations on canonical Debian
`0.0.1+http.1`, with freshly built `sillage-launcher` and
`sillage-extension-worker` and unchanged `sillage-search`. Its reported source
head differs from PR #561's actual head as recorded above; it is not evidence
for today's integrated source or current UI acceptance.

#### Operating constraints

1. **Existing Secret Service facility**: The host must provide an active user Secret
   Service implementation (e.g. GNOME Keyring) with an unlocked default collection.
   The launcher never prompts for master passwords, creates or unlocks keyrings,
   discovers unassociated application passwords, or falls back to plaintext storage.
   Missing or locked vaults fail closed with an explicit error view.
2. **Bearer-only authorization**: Only `Authorization: Bearer` is supported. Workers
   receive an opaque handle referencing host metadata; secret bytes are never
   returned to extension sandboxes. Unsupported schemes fail strictly at the worker.
3. **Scope and identity binding**: Every grant binds extension ID, validated package
   SHA-256, canonical HTTPS origin without trailing slash, method, exact path, and
   expiry TTL. Updating an extension package invalidates previously authorized handles.
4. **Network controls**: Requests enforce public IP validation, DNS pinning, strict
   redirect rejection (redirects are never followed), response size ceilings (16 KiB),
   and direct raw-token reflection rejection using native TLS roots exclusively.

The later #551 archival correction says the previously referenced original
archive is currently absent from reachable local evidence. A recovered
original remains publication-unaccepted because its receipt included two
generated endpoint-host identity fields; only a separately reviewed redacted
derivative was accepted for archival use. The historical source worktree is
unavailable; retained frozen source was validated instead. `package_outcomes`
still names producer-first, while the correction binds producer-second; preserve
both identities rather than reconciling them silently. These first outcomes
remain private and unchanged, not a rerun or current-source product acceptance.
This does not qualify desktop Wayland, multi-monitor/suspend, or aggregate
release gates.


### UI-only first-run document search

For the combined install, open the desktop entry and set up the activation
shortcut through the offered UI. Open **Document search** from its setup entry,
or Preferences with Ctrl+Comma. The native chooser requires
`xdg-desktop-portal` and a desktop FileChooser backend (GTK, GNOME or KDE);
the launcher Debian dependencies declare them. The launcher does not embed that
backend or install the optional search component on demand.

Choose a local folder, inspect its displayed canonical path, and select
**Enable document search**. Choosing a folder alone does not configure or start
search. Enable approves exactly the displayed root, creates a private managed
profile under `${XDG_DATA_HOME:-$HOME/.local/share}/sillage/launcher-search`,
creates its bounded search/evidence credential, and starts standalone
`sillage-search`. Private directories are mode 0700 and credentials mode 0600.
No credential entry, manual grant/init command or launcher settings edit is
required. Existing unmarked profiles are not adopted or chmodded.

The UI shows **Indexing** with indexed/pending counts. **Ready** requires a
completed durable scan, zero pending work and no indexing error. Close
Preferences, search document text, and press Return to inspect the actual cited
passage. Copy and source-open actions reopen current evidence under the grant;
the default viewer may not jump to the cited line or page.

Saved explicit managed consent resumes on later launcher starts. It does not
enable login autostart or start a model. Choosing a replacement folder retires
the old managed approval and requires new confirmation. **Disable document
search** clears launcher configuration and visible document content, revokes the
owned authority and stops the owned child within one absolute five-second
deadline. Failed cleanup is shown as an error rather than successful disablement.
Application launching, shortcut reactivation and arithmetic remain usable.
Selecting a folder does not change an existing external connection; explicit
Disable disconnects only the launcher client and does not stop, revoke or modify
that external provider.

A fresh private source-built X11 run passed the complete 240-Markdown-file
workflow, including native selection of a comma/space folder, visible Indexing
then Ready, Return, exact fresh copying, private default source-handler dispatch,
saved-consent restart, Disable and real app launch/shortcut/calculator use after
disablement. Both normal launcher shutdowns and namespace cleanup passed the
five-second gates without explicit force-kill. These are functional source-route
observations, not current installed-package, Wayland, reference-hardware, paint,
p95, relevance or release qualification. Earlier first outcomes remain preserved,
including the 1,800-file timeout at 120 s with 952 indexed and 848 pending.

A subsequent native Ubuntu 24.04 build produced current Debian launcher/search
packages with a verified GLIBC 2.39 ceiling, exact current runtime dependencies,
independent component payloads and mode-0755 executables. APT installed them in
a private runtime root separate from the compiler environment. A fresh X11
session verified installed package status/version/architecture and exact producer
hashes, then completed the full UI workflow using ordinary `/usr/bin` executables
and native GTK/GIO infrastructure, without explicit loaders or replacement
product/GIO wrappers. Disable took 178.081450 ms; enabled/final normal shutdowns
took 83.501040/53.013819 ms, with every five-second absence gate passing and no
force-kill. These are functional observations, not performance samples or
AppImage, Wayland, reference-hardware, relevance, Hybrid or release qualification.
The prior installed session passed all UI actions but failed private D-Bus group
cleanup after its leader exited; that first failure and its consumed inputs stay
closed. A fresh recipe stopped surviving owned group members within the same
absolute cleanup deadline. No product deadline or frozen qualification was changed.



The Debian package IDs are `io-github-briolabs-sillage-launcher` and
`io-github-briolabs-sillage-search`; the launcher binary/desktop identity
remain `sillage-launcher` and `io.github.briolabs.Sillage.Launcher`.

This is a breaking technical-identity cutover: the Maestria executable,
packages, IDs, environment variables, and paths have no Sillage compatibility
aliases or automatic migration. Old settings, grants, compositor bindings,
instances, credentials, and model assets remain where they are and are not
imported. No retention guarantee across the rename has been verified. The
legacy upgrade evidence later in this document covers only the old technical
identity; the rename alone does not clear the current retrieval or release
gates, and PR #516 remains a draft. The real GitHub/GHCR endpoints and their
historical evidence links retain their existing slugs.
The canonical profile gate starts the current launcher with its preference
file absent, observes actual first-launch defaults before configuring them
through the UI, restarts to verify the saved settings, and then tests current
remove/reinstall persistence while preserving the old profile. The first
current CI attempt (`37129751284`) failed indexing and did not retain the final
consumer failure payload; its exact runtime cause remains unproved.

A separate real source-built launcher/helper smoke passed the genuine defaults
and UI-save phases, preserved the private old-profile seed byte-for-byte at
mode 0600, and kept the private clipboard unchanged. Both public trees had no
property/walk errors; screenshots were viewed and every shutdown gate passed.
The private D-Bus service uses the existing read-only guest GSettings schemas;
its first missing-schema infrastructure failure and subsequent preparation
refusals remain preserved. This does not qualify actual package coinstallation,
restart persistence, or removal/reinstall.

A distinct local Ubuntu lifecycle passed in 14.37 s using all six authenticated
Debian artifacts from legacy CI `36455626998` and current CI `37129751284`.
The repaired caller initializes independent old/current search instances,
fixtures, credentials and grants; the old credential receives `Unauthorized`
at the current endpoint. Old launcher/extension/search/credential bytes and
modes remained unchanged through fresh current defaults, UI configuration,
current-only remove/reinstall, restart and final current revocation.
No old grant, profile, database or credential is imported into Sillage.

AT-SPI action acceptance is not state-transition completion: after the single
shortcut-defer action, the profile observer waits up to five seconds for the
offer to disappear without retrying the action or weakening defaults checks.
Normal product shutdown requires exit 0 and absence within a shared monotonic
five-second deadline; cleanup cannot establish success through force-kill.
Preparation removes inherited product packages only from the disposable
guest overlay, preserving the read-only support root. First preparation and
GUI outcomes remain retained, not replayed.

This is local exact-package evidence with source-caller/helper snapshots, not
successful overall-CI/reference release qualification. Search-package smoke
passed in `37129751284`; native finished 797/800 and the profile job failed.
The strict CI reference gate remains unpassed: 15.615 GiB effective memory
was below 16 GiB and SSD provenance was unestablished. Local isolated runtime
is the repair feedback loop; no runner provisioning, CI rerun or gate waiver
is inferred. Serving remains Shadow and PR #516 stays draft.

Installing either package does not start the search daemon, install a service
unit, or enable login autostart. The launcher remains useful for application
search without a search package or daemon. The search package is headless and
can own an index without installing the launcher.

For a native package smoke, run `scripts/smoke-launcher-packages.sh` inside a
private `dbus-run-session` and `xvfb-run`, with an outer timeout so `xvfb-run`
is not PID 1 of a container. The script enables the private session's
`org.a11y.Status.IsEnabled` before launching Slint; without that signal
AccessKit does not expose its AT-SPI tree even when the X11 window is visible.
The Ubuntu Debian and AppImage passed this bounded local smoke, including
first-run offer/deferral and shortcut persistence. Hosted Ubuntu 24.04 CI
[run 36340469343](https://github.com/brio-labs/maestria/actions/runs/36340469343)
rebuilt both formats from commit `a74c4cda`, installed the Debian package and
passed its native window smoke. This does not verify live desktop portal
approval, Wayland passage actions or a version upgrade.

The launcher-only package smoke requires the launcher Debian to be installed
at the package version under inspection, checks that neither the separate
search nor extension-worker package or executable is installed, and uses only
system paths for all launcher interactions. Run it on a clean launcher-only
Ubuntu installation; the combined-install smoke has a different purpose.
The absence/PATH assertions passed the launcher-native job of hosted Ubuntu
[run 36340469343](https://github.com/brio-labs/maestria/actions/runs/36340469343)
at commit `a74c4cda`; rebuild and rerun after subsequent code changes.
The Debian verifier independently checks that no launcher package relationship
pulls in, conflicts with, or claims the separately optional components.

On a pure Wayland session with no `DISPLAY`, the launcher uses the
`wl-clipboard` runtime dependency to copy text through the standard Wayland
clipboard protocol; its short-lived `wl-copy` parent exits after establishing
clipboard ownership. Text over 64 KiB is rejected before launching `wl-copy`;
nonblocking pipe writes and parent acceptance have one 500 ms post-spawn
deadline and report errors on timeout. The package still does not start a
search daemon.

A **source-built** Slint launcher exercised the real KDE GlobalShortcuts v2
portal on a private nested KWin Wayland compositor inside Xvfb, with private
D-Bus, XDG directories and the launcher's actual desktop-entry identity.
Selecting “Set Up Shortcut” called `CreateSession` and `BindShortcuts` for
`activate-launcher` with `CTRL+space` and displayed KDE's real consent dialog.
In one fresh private session, approving “OK” returned portal response `0`
with `Ctrl+Space` and persisted `shortcutSetup = "requested"`. In a separate
fresh session, rejecting “Cancel” returned response `1`, closed the session
and wrote no shortcut preference. KDE's dialog was activated through AT-SPI;
the launcher Setup button was clicked in the **private** nested X11 display.
Neither session used the live desktop or simulated a portal response. This
source-built evidence alone did not prove the packaged Ubuntu Wayland shortcut
path, permission retention across an actual version upgrade, or accessibility
for the launcher's Copy button.

On [run 36454007556 at `4962a0d7`](https://github.com/brio-labs/maestria/actions/runs/36454007556),
the **same-run Ubuntu 24.04-built launcher Debian** installed on Ubuntu
26.04 passed separate fresh, private KDE Wayland consent sessions without
installing search or the extension worker. The real KDE backend received
`CreateSession` and `BindShortcuts` for the registered
`io.github.briolabs.Maestria.Launcher` identity (the pre-cutover ID) and
`activate-launcher` / `CTRL+space`. Approval returned the matching Bind response `0` with
`Ctrl+Space` and retained `shortcutSetup = "requested"` after launcher quit;
denial returned `1` and left settings absent. The launcher Setup and KDE
dialog were both activated by the **private Xvfb pointer, not AT-SPI**.
This installed Ubuntu 26.04 decision does not prove stock Ubuntu 24.04 portal
support, compositor activation, version-upgrade retention or the first
installed extension Copy AT-SPI action.

Stock Ubuntu 24.04 cannot run this unsandboxed launcher's per-app Wayland
shortcut setup unchanged: it ships
[`xdg-desktop-portal` 1.18.4](https://packages.ubuntu.com/noble/amd64/xdg-desktop-portal),
while the host application
[`Registry`](https://github.com/flatpak/xdg-desktop-portal/blob/main/NEWS.md#changes-in-1194)
needed by `ashpd::register_host_app` first appeared in 1.19.4.
[KDE 5.27's backend advertises GlobalShortcuts](https://raw.githubusercontent.com/KDE/xdg-desktop-portal-kde/Plasma/5.27/data/kde.portal),
but the launcher requires host registration **before** `CreateSession`; do not
skip registration and silently lose its application identity. Ubuntu 26.04
provides a newer portal and KDE backend. The installed Ubuntu 26.04 approval
and denial proved above cannot establish shortcut availability on stock Ubuntu
24.04.

On stock Ubuntu 24.04, do not skip host registration to make the portal
request appear to work. Instead, if you want a global keybinding, explicitly
add a **compositor-owned** shortcut for `sillage-launcher --activate` in your
desktop's keyboard shortcut settings. On Ubuntu GNOME, use
[Settings → Keyboard → View and Customize Shortcuts → Custom Shortcuts → Add Shortcut](https://help.ubuntu.com/stable/ubuntu-help/keyboard-shortcuts-set.html.en#custom-shortcuts).
Choose an unclaimed key combination; do not install a second binding for the
same key. The launcher's Preferences → Copy activation command supplies the
command to paste, but does not configure the compositor. Remove the shortcut
through the desktop settings if no longer wanted; package removal does not
remove a compositor-owned binding. This is a manual activation fallback, not
a portal grant or proof of upgrade retention.

### Approve a search root and start the daemon

Choose and inspect a narrow, existing directory before approving it. The
`--read-root` argument is the explicit consent for the daemon to index that
directory; do not select a broad home directory unless its entire contents are
intended for indexing. The search package requires at least one approved root
when creating an instance:

```bash
INSTANCE="$HOME/sillage-search"
READ_ROOT="$HOME/Documents" # Replace with the exact directory you reviewed.
sillage-search init --instance-dir "$INSTANCE" --read-root "$READ_ROOT"
```

This stores the instance, index, credentials and watcher state under
`$INSTANCE`; the original files under `READ_ROOT` are not moved. To add or
remove a root later, keep the daemon running and use its owner commands:

```bash
sillage-search owner roots status --instance-dir "$INSTANCE"
sillage-search owner roots add --instance-dir "$INSTANCE" /absolute/reviewed/path
sillage-search owner roots remove --instance-dir "$INSTANCE" /absolute/reviewed/path
```

`add` and initialization reject a path that is not an existing directory or is
a symlink. Starting the daemon begins continuous indexing only for its approved
roots. Review the reported roots, exclusions and indexing freshness before
leaving it active. Hidden paths, symlinks, ignore files, and the built-in
privacy exclusions are skipped; these defaults do not replace choosing narrow
roots and checking the status output.

Run the daemon in a terminal where its lifecycle is visible:

```bash
sillage-search start --instance-dir "$INSTANCE"
# Stop it in this terminal with Ctrl-C.
```

`sillage-search start` uses the read-only profile and no model client. There
is no `sillage-search stop` command or package-installed service manager:
installation does not start it in the background. An operator-managed daemon
must be started explicitly; the UI-owned service starts only after explicit
Enable or saved managed consent. Root changes apply while the daemon runs;
after stopping an operator-managed daemon, restart it explicitly.

The daemon prepares the interactive source-version snapshot before announcing
its API socket; a large history may delay startup rather than consume the
100 ms per-request interactive deadline. The lexical-only refresh scans
source-changing events, not unrelated audits or grants; repository-code
security still needs full history. Watcher-state persistence uses compact JSON
compatible with older pretty-printed state. A watched edit or deletion
invalidates the snapshot. On an indexed 521-file private corpus, the first
changed-body request after a settled edit still hit the 100 ms daemon timeout
despite the narrower event scan; retrying once the snapshot was warm returned
the changed cited passage. Do not treat successful-only latency percentiles as
proof of live-indexing deadline acceptance.

On the subsequently changed **source-built** search-only daemon, an unmodified
100 ms request deadline and required durable audit accompanied 200/200 cited
socket-API searches during observation of 650 short Markdown files:
p50/p95/p99 29.70/35.97/43.80 ms, maximum 79.13 ms. The observer still had
610 pending files at the last interaction and reached zero 72 seconds later.
Fresh/edit/other-passage-after-delete classes also each returned 200/200
real excerpts. This does not establish native Slint timings, the retained
10,000-file deadline, a fresh Ubuntu package run or a provider-backed
French/English relevance gate. Preview revalidation refreshes source-event
truth when an unrelated file advances the global revision; it still denies
the cited source after its own edit/removal. The intermittent Tantivy startup
`LockBusy` in [#517](https://github.com/brio-labs/maestria/issues/517) remains open.

### Advanced external connection: scoped credential and launcher search

The UI-managed workflow above does not require these manual steps. For direct
headless search or an independently administered provider, issue an external
grant while the provider daemon is running. Use narrower `search-only` access
for bounded search previews; the launcher passage flow reopens evidence and
requires `search-and-open-evidence`. The example below uses that access, capped
at five internal results with 4 KiB evidence and a one-day grant:

```bash
INSTANCE="$HOME/sillage-search"
CREDENTIAL="$HOME/.config/sillage/search-client.key"
(umask 077; mkdir -p "$(dirname "$CREDENTIAL")")
consumer_realm="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
grant_policy=(
  --instance-dir "$INSTANCE" --consumer-realm "$consumer_realm"
  --access search-and-open-evidence --max-sensitivity internal
  --read-root "$READ_ROOT"
  --max-results 5 --max-evidence-bytes 4096 --expires-in-seconds 86400
)
sillage-search owner grant review-external "${grant_policy[@]}" \
  --consumer-label "Headless search client"
# Inspect provider, exact roots, access, bounds and TTL before approving.
sillage-search owner grant create-external "${grant_policy[@]}" \
  --credential-file "$CREDENTIAL"
stat -c '%a %n' "$CREDENTIAL" # The credential file must be mode 600.
```

Repeat `--read-root` for each root this consumer may read. Each path must
resolve to an **exact currently approved root**; a subdirectory is not a
separate approved root. Omitting the flag freezes **all currently approved**
roots into this new grant. The grant does not expand when the provider later
approves another root; removing a root denies that source even if it is still
listed in a grant. Search passages, filename-only paths, evidence opens and
consumer indexing inventory are limited to the granted roots before content
I/O. Provider observer progress still describes the entire provider.
Each grant is bounded to 64 roots and 8 KiB total root-path bytes.

`review-external` is a read-only owner operation: it checks current approved
roots and existing consumer grants but creates no grant or credential. Its
output omits realms, grant digests and credential paths. The required public
consumer label is display metadata, not authorization identity; for an
extension, use its reviewed extension identifier. Expiry is explicitly a TTL
starting at the subsequent owner creation action, not an absolute timestamp
bound during review. Reuse the same explicit root set and policy arguments for
creation; do not omit roots and silently approve a changed provider inventory.
Creation rechecks current authorization and may fail if provider state changed.

For renewal, revoke any unrevoked old grant, run a fresh review and explicitly
create the newly approved grant. Extension permission and provider-owner grants
remain independent: extension disable/revoke does not revoke the separately
owned provider grant, and owner grant revoke denies later provider requests
without uninstalling the extension. This local-provider boundary is
[#550](https://github.com/brio-labs/maestria/issues/550), not the remote HTTP
credential work in [#551](https://github.com/brio-labs/maestria/issues/551).

Pre-v18 grants retain legacy **all currently approved roots** semantics,
including roots added later. `owner grant list` labels them
`allowed_roots=legacy-all-approved`; newly issued grants print the frozen root
list. Revoke and reissue any legacy grant before adding a root its consumer
must not read. Root scopes cannot be edited on an issued credential.

The command prints the grant digest and consumer realm, but not the bearer
credential; it creates the credential file with owner-only mode `0600`. Save
the printed grant digest for later revocation. For a headless client, give
`sillage-search search` the provider socket, this realm and credential file:

```bash
sillage-search search \
  --socket-path "$INSTANCE/system/daemon.sock" \
  --consumer-realm "$consumer_realm" --credential-file "$CREDENTIAL" \
  --limit 5 "a phrase from an approved source"
```

To connect the launcher to that external provider, edit its user-owned schema-1
settings file at `${XDG_CONFIG_HOME:-$HOME/.config}/io.github.briolabs.Sillage.Launcher/launcher.toml`.
If the file does not exist, launch the app and make a first-run shortcut choice
(including “Not Now”) or save a preference so it writes the current settings.
Preserve its existing settings and `schemaVersion = 1`; add this table with
absolute paths and the exact 64-hex consumer realm printed by the grant command:

```toml
[search]
socketPath = "/home/alice/sillage-search/system/daemon.sock"
consumerRealm = "replace-with-the-64-character-hex-consumer-realm"
credentialFile = "/home/alice/.config/sillage/search-client.key"
```

Replace the example home paths and realm. The launcher requires an absolute
socket path, a 64-character hexadecimal realm and an absolute credential path.
Keep `sillage-search` installed and on `PATH`. Restart the launcher after
manual editing (`sillage-launcher --quit`, then `sillage-launcher --activate`).
**Disable document search** also disconnects an external launcher connection
without changing its provider. Alternatively, removing the optional `[search]`
table and restarting disables its passage requests without uninstalling either
package. Revoke the provider grant separately if access should end for every client.

In search results, Up and Down move between actionable rows and stop at the
first or last actionable result. Document-group headings are labels, not
buttons or keyboard selection targets. Return opens the selected cited
passage; it does not activate a heading. This also applies when application
or calculation results appear before document passages.

The package does not fetch model or OCR artifacts. Text and lexical search do
not require a model; the search daemon starts without a model client, and
scanned pages without OCR remain `NeedsOcr` rather than producing fabricated
text. OCR or visual/model use requires separately provisioning and explicitly
configuring a local provider; Sillage does not download or execute model code.
The current provider profiles in [RESEARCH.md](./RESEARCH.md) are dated
implementation candidates, not a release-managed package toggle. Do not set
`ocr_*` or `visual_*` manifest keys until the corresponding local provider and
its revision/artifact have been reviewed.

### Optional extension worker

The launcher does not depend on the worker. Install it only when choosing to
run extensions:

```bash
sudo apt install ./target/extension-packages/sillage-extension-worker_0.0.1_amd64.deb
```

Its package ID is `io-github-briolabs-sillage-extension-worker`; it brings
`bubblewrap` for the launcher's OS sandbox. Installing it alone does not install
an extension, grant extension capabilities, start a daemon, or execute extension
code. The launcher refuses to run extensions outside the sandbox if the worker
or sandbox setup is missing.

The new `extension-worker-package` Ubuntu job builds the worker separately,
installs exactly its declared runtime dependencies, verifies that launcher
and search remain absent, requires working bubblewrap user/pid/network
namespaces, runs an installed JavaScript command with a host-filesystem
canary inaccessible, and removes the worker. It does **not** prove a real
launcher broker grant, a version-different upgrade or the portal's permission
decision. Hosted Ubuntu [run 36350436748](https://github.com/brio-labs/maestria/actions/runs/36350436748)
at `6583bfae` passed all these installed-worker stages, not the real broker.

A freshly built local Ubuntu 24.04 worker Debian in
`target/extension-packages-ubuntu-current` passed apt installation, metadata,
installed-binary ownership and modularity checks in disposable Podman. The
smoke stopped at the required bubblewrap preflight: nested Podman denied the
`/proc` mount. No worker command was run outside the sandbox; the JavaScript,
canary and removal stages remain unverified locally. Hosted Ubuntu
[run 36349114236](https://github.com/brio-labs/maestria/actions/runs/36349114236)
also built and installed the worker but stopped before the JavaScript command:
AppArmor denied loopback setup in Bubblewrap's private network namespace.
The passing hosted job loaded Ubuntu's specific `bwrap-userns-restrict`
profile only on its disposable runner; it did not disable system-wide
namespace protection, share the network, or execute the worker outside
the sandbox. Unlike local nested Podman, hosted CI verified the real
JavaScript response, inaccessible host canary and clean worker removal.

An isolated source-built Slint launcher also displayed a real Copy-only
extension's version, package digest and exact new grant, then approved its
install. Its first broker invocation stopped before JavaScript: Bubblewrap's
namespace setup returned `Resource temporarily unavailable`. This desktop had
1,519 threads under the same UID, above the worker's 1,024-task
`RLIMIT_NPROC`; an otherwise identical namespaced `/usr/bin/true` preflight
failed with that limit and passed without it. Linux counts tasks for that
limit across the real user, not just the worker. The worker now keeps a bounded
4,096-task UID-wide ceiling, with its separate user/pid/network namespaces,
512 MiB address-space cap, 32-second CPU cap, file and descriptor caps
unchanged. A namespaced preflight passed with the new ceiling on that desktop.
In a fresh private X11 session the source-built launcher then installed the
reviewed Copy-only extension, ran its Bubblewrap-isolated worker, received a
denial for an ungranted file-search request, and wrote authorized text to the
private clipboard through the real host broker. After the launcher quit and
restarted, the same approved command again denied ungranted search and
completed the authorized Copy action; revocation then made its command
unavailable. These developer-build observations alone did not establish the
installed Ubuntu combined flow, version upgrade or packaged portal consent.
Never fall back to unisolated worker execution when a sandbox still fails closed.

The separate combined Ubuntu 24.04 package job passed at
[`adea9552`](https://github.com/brio-labs/maestria/commit/adea9552d487cf2d58e51daa9c3efe0006b4185b)
([CI run 36421328686](https://github.com/brio-labs/maestria/actions/runs/36421328686)).
It installed the **same-run** launcher and optional worker Debian artifacts,
verified their package ownership and the absence of optional search, and used
the actual installed Slint launcher with a private Xvfb, D-Bus, XDG home and
clipboard. The Copy-only extension's exact version, digest and permission
were reviewed before approval; its Bubblewrap-isolated worker was denied
ungranted file search and copied authorized text via the host broker. A real
launcher quit/restart retained the grant and repeated the denied search and
Copy; revocation removed the command. The job then removed only the worker
and verified that the launcher remained installed. No worker ran outside
Bubblewrap and no optional daemon was started.

This is **not** Ubuntu Copy-action accessibility acceptance. On private Ubuntu
Xvfb [run 36418631504](https://github.com/brio-labs/maestria/actions/runs/36418631504),
the visible, enabled Copy button rejected AT-SPI `do_action(0)`. The installed
Ubuntu [run 36424740250](https://github.com/brio-labs/maestria/actions/runs/36424740250)
exposed **zero** AT-SPI actions on the first visible Copy button, but `click`
on the second after restart; its broker smoke used a pointer for both.
Explicitly binding Slint's default action still left the first invocation
without an AT-SPI action in
[run 36426864440](https://github.com/brio-labs/maestria/actions/runs/36426864440);
that ineffective duplicate binding was removed; track the actual defect in
[#545](https://github.com/brio-labs/maestria/issues/545). The separate
installed Ubuntu 26.04 KDE portal approval/denial above is a real consent
decision, but neither Copy AT-SPI nor stock Ubuntu 24.04 portal support.

At [`f8949aa1` run 36472914403](https://github.com/brio-labs/maestria/actions/runs/36472914403),
the installed Ubuntu 24.04 combined broker instead **failed closed**: the
first-run Copy button's direct D-Bus `GetInterfaces` included `Action`, but the
GI/libatspi client cached only `Accessible` and `Component` and offered no
`click`. The publisher emitted malformed cache signal argument signatures;
the installed client did not refresh that interface. Do not count direct
publisher interfaces or pointer-backed Copy as client-side AT-SPI acceptance.

The locked AccessKit Unix AT-SPI cache signal body was corrected at `fbf52260`.
In [run 36475736114's installed Ubuntu 24.04 combined job](https://github.com/brio-labs/maestria/actions/runs/36475736114/job/109110091717),
the **first** visible Copy action appeared as GI/libatspi `click`, and
`Atspi.Action.do_action(0)` activated it without a pointer. The installed
Bubblewrap worker denied ungranted search, copied the exact private-Xvfb
clipboard text, retained the grant after launcher restart, and lost the
command after revocation. [#545](https://github.com/brio-labs/maestria/issues/545)
is closed on that exact installed proof; it does not prove upgrade retention
or reference-hardware performance.

The installed version-different upgrade passed at `0965269c`. The separate
10,000-file/reference-hardware latency and resource gates remain open.

### Reproduce release acceptance in isolated CI runners

The `version-upgrade-package` job downloads the successful **0.0.0** launcher,
search, and worker Debian artifacts from
[run 36455626998 at `985a8368`](https://github.com/brio-labs/maestria/actions/runs/36455626998)
and all three **0.0.1** artifacts built from the same new CI checkout.
`scripts/smoke-version-upgrade.sh` refuses identical upstream versions,
unchanged installed executable bytes, or a version-only source change. On an
Ubuntu 24.04 runner, it installs the old packages, creates one private XDG
launcher/extension/grant/data tree and one private authorized search instance,
then replaces all three packages. It compares persistent state **before**
restarting either process, checks authorized evidence and denied roots/consumers
after restart, removes and reinstalls the new packages without reconstructing
that state, and checks retention again before explicit revocation. Its Xvfb,
D-Bus, and clipboard belong to the job; the worker still requires Bubblewrap.
The procedure alone does not establish that an upgrade has passed.

At that same `f8949aa1` run, the old **0.0.0** packages installed, but the
old-version search check stopped before upgrading. The test rejected all
passage hits for an ungranted-root phrase, including hits that could cite the
approved file. The revised boundary assertion checks each result's source path
and excerpt for unauthorized content; this run did not perform an upgrade.

At `fbf52260`, old-package search and the installed 0.0.0 Bubblewrap
extension's AT-SPI Copy completed before the upgrade; the job then stopped
because its storage snapshot expected a file directly under the extension's
data directory. The old broker actually creates a second extension-ID
subdirectory beneath it. No new packages were installed in that run; the
corrected snapshot was exercised by the next installed run.

At [`0965269c` run 36478888264](https://github.com/brio-labs/maestria/actions/runs/36478888264/job/109120534027),
the Ubuntu 24.04 `version-upgrade-package` job **passed**. It installed
different upstream versions (all three `0.0.0` packages, then the same-run
`0.0.1` packages), verified changed product code and installed binary hashes,
restarted the private search daemon/launcher with the same settings,
root-scoped search credential, and Bubblewrap extension storage/grants, then
removed and reinstalled the packages while retaining that state. The final
checks explicitly revoked search and extension access. This is installed
upgrade/retention evidence, not a same-version reinstall or source-built test.

The separate `installed-native-benchmark` job installs the same-run Debian
launcher and search binaries. It records 200 actual native-window/AT-SPI
interactions each for cold launcher starts, active queries, edited sources, and
deleted sources over one approved 10,000-file tree. It retains failures and
timeouts, exact artifact/process identity, resource samples, screenshots when
captured, citations, and actual memory/SSD provenance in the
`installed-native-benchmark-evidence` artifact, even if the gate fails. A runner
below **16 GiB effective memory** or without confirmed SSD storage cannot pass
the reference-hardware gate. Keyboard-to-AT-SPI measurements include observer
overhead and **do not** directly measure or replace the internal 100 ms search
deadline. Neither job should be run against a user's desktop or package state.

The installed native benchmark has not passed; successful installed samples
on qualifying reference hardware are still required.

The `f8949aa1` native benchmark stopped at daemon startup because its private
Unix socket path under the long CI workspace exceeded `SUN_LEN`. Its artifact
also measured **15.619 GiB effective memory** and rotational backing devices;
that runner cannot pass the reference hardware gate even after a shorter
private socket path permits samples to run. Obtain installed measurements on a
verified SSD with at least 16 GiB effective memory before claiming release
latency/resource acceptance.

At `fbf52260`, the shorter socket let the installed search daemon run. The
10,000-file watcher reported **7,200 indexed, 2,800 pending** after 900 seconds
with no last scan error. Resource evidence recorded about 8.26 GB of daemon
disk writes and a 461 MB resident high-water mark; UI samples never started.
The indexing settle allowance is now 1,800 seconds without changing the
10,000-file requirement or the 100 ms internal interactive-search deadline.
This runner still lacks the required effective memory and verified SSD.

At `0965269c`, the installed benchmark settled all **10,000 approved files**
with zero pending and no last scan error after about 1,250 seconds. It recorded
200 observations in each of the four classes, but **all 800 failed**: cold
and active samples found the result button but could not find its enclosing
`Launcher results` list; the first edit/delete precondition failed likewise,
and subsequent edit/delete attempts could not focus the hidden window after
Escape. These are failure-inclusive timings, not successful-result latency.
An isolated source-built Slint/AT-SPI diagnostic exposed the named list as
`LIST_BOX`, while the installed driver's predicate required `LIST`. The
driver now matches the observed role and avoids hiding an already-focused
results view; a private source-built two-query UI smoke exercised that change,
not the installed 10,000-file gate. The hosted runner also remained below
16 GiB effective memory with rotational storage, so it cannot establish
reference-hardware acceptance even if the corrected UI samples pass.

At [`e81d44f3` run 36486876128](https://github.com/brio-labs/maestria/actions/runs/36486876128/job/109146574780),
the installed benchmark again settled **10,000 approved files**, zero pending,
with no scan error, and recorded 200 observations per class. All **800 timed
out**. Every cold query observed a native accessible result; 92 cold
interactions opened its detail and verified the **exact authorized citation
and excerpt** through accessible Copy, but then timed out because keyboard
Escape did not return to the search entry. The remaining 108 cold interactions
timed out awaiting detail rows or Copy controls: the detail card was visible
in the private Xvfb capture while its dynamic AT-SPI child was sometimes
missing or returned an unknown object path. Active/edit/delete attempts then
failed to open or recover from the detail view. These are failed interactions,
**not** successful latency samples. In a separate private, source-built
one-file diagnostic, invoking the genuine AT-SPI “Return to launcher results”
button closed the detail, restored focused search, and permitted the next
typed query; that is not installed acceptance. The hosted runner confirmed
SSD storage but had only **15.615 GiB effective memory**, below the required
16 GiB. Its resource sampler also failed with `PermissionError` reading
`/proc/25178/io` for a short-lived launcher; the resource-telemetry gate
failed independently. Keep the benchmark and reference-hardware gates open.

A private D-Bus trace reproduced the missing detail as a provider-side
lifecycle fault: the same AT-SPI object path received `AddAccessible` for the
passage, then `RemoveAccessible` **0.843 ms later** while the card remained
visible. A fresh client and direct provider introspection both found the path
absent. The Unix accessibility bridge now reconciles queued node registration,
removal, and cache events with the current visible tree before changing that
path; the same-ID replacement regression passed. A rebuilt **source**
launcher completed eight authorized passage-detail opens, exact citation and
excerpt Copies, accessible returns, and subsequent typed queries in one
private Xvfb/D-Bus session; this does **not** replace an installed benchmark
run. The benchmark uses the accessible Return button for navigation after
Copy. The separately reproduced keyboard Escape failure arose when opening
detail hid the focused query entry. Explicitly focusing the native
`FocusScope` on open restored **actual keyboard Escape** before and after
exact authorized Copy, plus the next typed query, in another private
source-built Xvfb/D-Bus smoke; installed verification remains open. The
resource monitor discards a `/proc` permission race only if the exact
registered process has exited or been replaced; a live target's permission
failure remains fatal.

At [`07709545` run 36510002235](https://github.com/brio-labs/maestria/actions/runs/36510002235/job/109220406839),
the installed binaries settled **10,000 approved files** with zero pending and
recorded all **200 observations per class**. Cold Copy passed 198/200; two
cold queries reported “Document search unavailable.” Active Copy passed
18/200; sample 18 timed out waiting for its Copy notice, and subsequent typed
queries retained the old text and timed out. In a separate private **source**
reproduction, a catalog revision refresh closed an open detail without
restoring search-entry focus. All edit/delete interactions failed or timed
out before reaching a fresh result: whenever their precondition Copy succeeded,
it left the private clipboard containing the excerpt, but the benchmark
compared it with the preceding citation and falsely reported that
stale-evidence denial changed the clipboard.
The durable source edits/unlinks and denied stale-action notices did **not**
establish an unauthorized copy. All 800 durations include failures; the
active/edit/delete p95 values are not successful latency evidence, and no
edit/delete sample reached fresh-result observation. Resource telemetry passed
with **44,018 samples** and no sampler error. This runner had **15.615 GiB**
effective memory and could not establish SSD backing from `lsblk`;
reference-hardware acceptance remains open.

The driver now compares the stale-action clipboard with the **last actual
pre-mutation Copy**, the excerpt, without weakening denial or fresh-source
checks. The launcher defers an unrelated catalog-revision re-search while a
passage detail is open, preserving its accessible Copy and Return controls;
the pending revision is applied after returning to results. A separate private
**source-built** Xvfb/D-Bus run on 202 generated approved files completed
**200 consecutive** genuine keyboard-open, exact citation/excerpt Copy, and
accessible Return interactions, followed by one edited and one unlinked
stale-evidence action denied with the unchanged private clipboard. This is
not a changed-commit installed 10,000-file acceptance run.

At [`59079579` run 36520546866](https://github.com/brio-labs/maestria/actions/runs/36520546866/job/109252741236),
the exact same-run installed launcher/search binaries settled **10,000 approved
files**, zero pending, and recorded all 800 interactions. Cold Copy passed
195/200, active Copy 198/200, edit 3/200, and delete 77/200. The **327**
failed observations include five cold and two active queries returning
“Document search unavailable”; after durable edits, stale evidence was denied
but most fresh queries timed out with unavailable search or no fresh result.
Delete failures likewise include unavailable search before or after unlink.
No failed or timed-out duration is successful-result latency. Resource telemetry
passed with **39,054 samples**, but the hosted runner had only **15.615 GiB**
effective memory; its SSD gate passed. This installed run failed both the
interaction and reference-hardware gates.

The same `59079579` Debian artifacts were independently installed by `apt`
inside a private Ubuntu 24.04.5 rootless namespace on local NVMe-backed
storage. Both `/usr/bin` executables matched their `.deb` hashes and `dpkg`
owners; nested Bubblewrap user/PID/network isolation passed. The private
Xvfb/D-Bus run settled **10,000 approved files** and recorded 200 observations
per class: cold Copy passed 199, active Copy 200, edit 17, delete 91.
Its **293** failures/timeouts again concentrated on unavailable document search
or fresh edit results after durable mutation; stale edit/delete evidence was
denied. The **30.854 GiB effective memory**, single-device Btrfs filesystem
backed by an `lsblk`-verified NVMe disk, and **36,405** valid resource samples
passed the local hardware/telemetry gates. Local execution used a worktree
benchmark driver with Btrfs source resolution; it is **not** an exact-head CI
script run, and its failed interactions do not establish installed release
latency acceptance. The Btrfs resolver now also requires the kernel's
`/sys/fs/btrfs` member list to confirm exactly one backing device, because
`findmnt SOURCES` alone can omit members in a rootless namespace; a separate
post-change installed-package hardware probe passed without repeating the
10,000-file workload. Keep the native performance release gate open.

A separate private installed-daemon **diagnostic**, not a release benchmark,
settled 2,000 authorized files and durably edited 90 of them. All 90 immediate
direct searches for the old term and all 90 for the new term returned typed
`NoEvidenceFound`, without CLI errors; the pre-edit warmup did find its
authorized passage. This confirms asynchronous indexing can leave a new term
unavailable at the first immediate query. It does **not** explain the many
“Document search unavailable” responses in either 10,000-file run or
establish when each changed file became searchable. Do not turn their failed
observations into passing latency measurements.

An additional **diagnostic**, not a release benchmark, isolated the service
error behind some mutation failures. Its attempted 10,000-file fixture was
interrupted; cleanup removed 1,527 generated files, so only **8,473** remained
approved and indexed. With the installed `59079579` search binary, 40/40
unchanged-file direct queries found their passages. After eight separate
durable edits, each first fresh query **after indexing settled** returned
`DaemonUnavailable: search service request timed out` at 103–104 ms of CLI
wall time. A subsequent diagnostic query found each fresh passage, while
the old term stayed absent. This distinguishes post-revision rebuild timeouts
from the separate pre-index `NoEvidenceFound` response.

The source runtime now replays only source events added since the cached
revision, preserving the same active-version/stale-source projection and the
unchanged 100 ms server timeout. A rebuilt, optimized **source** search binary
on the retained private 8,473-file instance found fresh evidence on the first
post-index query after eight different edits (41–52 ms CLI wall time); all
eight old terms stayed absent. Immediate pre-index fresh queries still returned
`NoEvidenceFound`. This is not installed 10,000-file/200-per-class UI
acceptance, does not retroactively repair failed observations, and does not
establish a fresh-result p95 for the qualifying installed release gate.

At [`5c28e044` run 36533424795](https://github.com/brio-labs/maestria/actions/runs/36533424795/job/109292924732),
the **installed** launcher and search packages settled 10,000 approved
files and recorded all 800 fixed interactions. Cold passed **199/200**,
active **200/200**, edit **96/200**, and delete **200/200**. The **105**
failed/time-out observations include one cold and 104 edit attempts. Of the
104 edit failures, 101 reached durable mutation, denied reopening the stale
evidence, and issued the single fresh UI query; **100** edit errors contained
the AT-SPI no-matching-passages status, while four contained “Document search
unavailable”. The launcher had no source-event-triggered re-search: a
`NoEvidenceFound` display persisted until another user/catalog change.
The edit class's failure-inclusive query-to-observation p95 was
5,081 ms, **not** successful-result latency; the 96 observed fresh results'
source-change p95 was 4,214 ms. Resource telemetry passed (28,834 samples),
but effective hosted memory was **15.615 GiB**, below 16 GiB. The installed
benchmark and hardware release gates still fail; neither the source-built
8,473-file diagnostic nor this installed failure establishes acceptance.

At `9982a322`, the native launcher first observed a
**search-grant-authorized** durable source-event clock while visible. A source
advance restarted the entire current query and cleared its existing results
and accepted actions; an unchanged no-match response was not timer-retried.
It deferred refresh while a passage detail was open or input was debouncing.
The server's **100 ms internal interactive-search deadline was unchanged**.
An optimized **source-built** launcher and
search binary in private Xvfb/D-Bus/XDG rendered a no-match result for one
unchanged query, then, after a durable edit in one approved Markdown file,
automatically displayed the fresh cited passage on the same query through
AT-SPI in **907 ms** from the edit; the old term returned no match. The
authorized clock moved from revision 11 to 24. This is a single-file
source-built native UI smoke, not an installed 10,000-file performance pass,
and does not erase any earlier failed observation.

At [`9982a322` run 36542027669](https://github.com/brio-labs/maestria/actions/runs/36542027669/job/109320552088),
the same-run **installed** Ubuntu packages again settled 10,000 files and
recorded every one of the fixed 800 interactions. Hosted cold passed
**200/200**, active **199/200**, edit **189/200**, delete **186/200**: **26**
failures/timeouts, including missed pre-deletion results/details and missed
post-edit details. Its effective memory was **15.614 GiB**, below the 16 GiB
reference gate, although SSD provenance passed. The identical `.deb` artifacts
were explicitly reinstalled inside a private Ubuntu 24 rootfs on local
**30.854 GiB** hardware. That independent installed run also recorded 800:
cold **200/200**, active **200/200**, edit **198/200**, delete **192/200**;
the **10** failures include eight pre-deletion result/detail failures, one
pre-edit missing result, and one post-edit “Document search unavailable.”
Its hardware gate **failed**: an outer private `tmpfs /tmp` held the fixture,
and a synthetic `/dev` hid the NVMe-backed Btrfs root's LVM child from
`lsblk`. Evidence remains under
`target/ubuntu24-private/rootfs/bench/current-head-9982a322-first-valid/`;
the earlier aborted preflight has separate provenance and **no interactions**.
Neither installed run is a release pass; failed durations remain in the
reported p95/p99 distributions, not in successful-result latency.

The launcher now refreshes **only the authorized passage results** on an
observed indexed source change. It leaves the existing query's accepted
actions and visible rows in place while fetching, does not re-render an
unchanged passage/path response, and defers applying a changed response while
a detail or file selection is open or input is debouncing. This removes the
destructive full-query refresh window seen at `9982a322` without retrying
failed interactive requests or changing the server deadline. A separate
optimized **source-built**, two-file native Slint/AT-SPI smoke used private
Xvfb, JWM, XDG, and D-Bus configured to autostart **only AT-SPI**, not optional
portals: the original cited passage stayed selectable in **36** observations
while the unrelated file's clock advanced **19 → 34**; its open detail survived
another unrelated edit (**34 → 46**); and an initially empty unchanged query
showed a fresh cited passage and opened its detail after its source edit
(**46 → 52**). A separate *non-workload* run of the unchanged provenance
checker at
`target/ubuntu24-private/rootfs/bench/hardware-preflight-corrected-9982/`
verified the exact installed `9982a322` binary hashes, **30.854 GiB**
effective memory, real Btrfs for both fixture/output, and the NVMe → LVM
`lsblk` chain; both hardware gates passed **only in this corrected
preflight**, not in the earlier 800-interaction run. The new launcher behavior
remains **source-built smoke, not installed 10,000-file acceptance**.

At [`d4eccf7b` run 36550481561](https://github.com/brio-labs/maestria/actions/runs/36550481561/job/109348121080),
the repaired, same-run installed Ubuntu 24 packages settled all 10,000 files
and recorded all 800 observations. Hosted cold, active, edit, and delete
passed **200/200**, **200/200**, **195/200**, and **200/200** respectively:
five edit observations timed out. Hosted effective memory was **15.614 GiB**
and SSD provenance could not be established, independently failing the
reference-hardware gate. Its complete artifact is retained at
`target/benchmark-d4eccf7b-ci/`.

An independent installed run of those **same-run** Debian artifacts in the
private Ubuntu 24 rootfs used the corrected Btrfs fixture/output and
read-only NVMe/LVM mapper and udev device evidence. Both exact package
payload hashes matched the installed binaries; the hardware gate passed
with **30.854 GiB** effective memory and confirmed SSD backing. The indexing
preflight settled 10,000 files with none pending and no scan error. All 800
interactions completed: cold **199/200**, active **199/200**, edit **175/200**,
delete **192/200**. The **35** timeouts mostly showed “Document search
unavailable” instead of the expected cited result or no-match status. All
failure-inclusive durations and telemetry remain at
`target/ubuntu24-private/rootfs/bench/current-head-d4eccf7b-first-valid/`;
the preceding package/hardware-only check is separate at
`target/ubuntu24-private/rootfs/bench/hardware-preflight-d4eccf7b/`.
**Neither installed run passes** the release gate. The failed workload is
not rerun to seek a pass, and its right-censored timeout durations are not
reported as successful-result latency.

The same `d4eccf7b` installed Ubuntu 24 launcher binary
(`44236221e9559ca1b95130445c7220a43a0f7950ddd3377755b954640697e25a`)
also completed a separate **native Open File selection** under rootless,
read-only Ubuntu 24 Bubblewrap in a private headless Cage Wayland session.
The **Arch GTK portal**, on an isolated session bus, returned
`org.freedesktop.portal.Request.Response` with success code `0` and the
approved throwaway `file:///tmp/sillage-cage.6nqsMm/only-approved-chooser-file.txt`;
the installed Slint launcher displayed that exact selected-file path. The
private result, screenshot, and verbatim successful portal response are retained
at `target/wayland-chooser-smoke/evidence-sillage-cage.6nqsMm.json`,
`target/wayland-chooser-smoke/selected-file-sillage-cage.6nqsMm.png`, and
`target/wayland-chooser-smoke/installed-portal-response-sillage-cage.6nqsMm.txt`.
This proves installed launcher selection with **Arch GTK**, not stock Ubuntu
24 GNOME's chooser or compositor-owned shortcut; it does not change the
failed installed performance gate above. The subsequently coordinated
launcher searches are source changes and are **not** present in that package.

The subsequent source-built launcher change serializes interactive searches
for one consumer realm across foreground queries, indexed-source refreshes,
and fresh path verification. A newer foreground generation cancels its
predecessor; source refresh waits until the foreground result has applied.
An optimized **source-built**, private two-file Cage/AT-SPI smoke observed a
real approved source revision **19 → 34**, returned the fresh citation from
`source-two.md` for a new foreground query, and kept that query and result
visible over the next two seconds of refresh ticks. Its evidence and private
screenshot are retained at
`target/wayland-chooser-smoke/evidence-search-sillage-search-ui.jySQUI.json`
and `target/wayland-chooser-smoke/selected-search-sillage-search-ui.jySQUI.png`.
The smoke did not force a deterministic request collision and does **not**
attribute all 35 installed timeouts to that race. The daemon's **100 ms**
interactive deadline and every installed benchmark gate remain unchanged.

At [`698fa6eb` run 36572828874](https://github.com/brio-labs/maestria/actions/runs/36572828874/job/109421722057),
same-run installed Ubuntu 24 launcher and search packages completed all
**800** hosted observations after this coordination change: cold **198/200**,
active **200/200**, edit **197/200**, delete **200/200**. Two cold and two
edit queries timed out with “Document search unavailable”; a third edit
observation could not open its precondition citation. All five remain
failures, not successful latency measurements. Package ownership and payload
hashes matched the installed binaries; the hosted machine independently
failed reference hardware at **15.614 GiB** effective memory, despite
established SSD backing. The complete failure-inclusive artifact is at
`target/benchmark-698fa6eb-ci/`, with exact same-run Debian packages at
`target/ubuntu24-private/current-head-698fa6eb/`. This head **fails the
interaction and hardware gates**; a local replay of its failed installed
workload is not performed to seek a pass. “Document search unavailable”
still conflates process, parsing, supersession, and daemon-timeout failures;
this run did not capture a per-stage cause.

A subsequent bounded **installed `698fa6eb`** native edit diagnostic recovered
three unchanged queries after indexing, but a one-MiB synthetic edit remained at
`parser_started` for 60 seconds. An isolated **source-built** reproduction
attributed **92.41%** of CPU-core cycle samples to `DocumentTree::new`: its
validator repeatedly traversed each link chain and linearly searched the node
array at every hop. The public and domain completion paths now share indexed,
memoized tree validation. Duplicate IDs, invalid roots, dangling links, both
cycle kinds, and atomic rejection of parser records remain enforced.

Removing that stall exposed a separate publication-clock failure. With the
installed launcher and a source-built repaired daemon, a one-MiB edit reached
durable `artifact_indexed`, but the unchanged query remained `NoEvidenceFound`
through three later clock replies: each still reported tree-capture revision
**6449**. The authorized consumer endpoint now advances on durable
`full_text_indexed`, emitted after index commit/reload, and final
`artifact_indexed` readiness, as well as source changes. The source-snapshot
clock remains source-only; publication and query audits do not rebuild source
projections or cause self-refresh loops.
A distinct one-MiB native case then showed the fresh `note-06.md:1-3` citation
without retyping, with the exact query retained after completed indexing.
These are private stock-GNOME/AT-SPI diagnostics using the **installed
`698fa6eb` launcher plus a source-built daemon with Arch libraries**, not
installed Ubuntu end-to-end acceptance. Transient typed deadline errors during
incomplete indexing remain observed; neither repair attributes the historical
CI failures or identifies the historical `LockBusy` writer. Authorization,
fresh evidence reopening, durable audit, Bubblewrap isolation, the **100 ms**
deadline, and all release gates remain unchanged.

The published `ddf9bcb8` installed acceptance remains **failed**: all 800
observations were retained, including 45 timeouts. Its separate installed
native edit diagnostic observed publication before the unchanged fresh-query
refresh, which still hit the internal deadline. Neither workload was replayed.

A later private 10,000-file diagnostic used the installed `ddf9bcb8`
launcher/CLI and a **source-built daemon linked to Ubuntu runtime libraries
with the host Rust/compiler toolchain**, not the CI-produced daemon. Its first
cold unique-token request timed out: filtering took **36.94 ms**, previews
**42.06 ms**, and the serve future was cancelled during durable audit at
**101.77 ms**. Separate evidence-stage measurements found two corpus-wide
watcher-state loads, **9.93 ms** and **8.67 ms**, inside the two source
freshness checks; full-text reader assembly took only **1.49 ms**.

Evidence reopening now rehashes the authorized source bytes at both freshness
checks instead of deserializing the complete watcher state. Bulk status checks
retain watcher-signature caching; root/privacy/symlink checks, fresh reopening,
durable audit, isolation, and the **100 ms** deadline are unchanged. This
trades corpus-wide metadata replay for work proportional to the opened source.
With temporary timers removed, the first repaired source-built native attempt
returned the formerly failing cold query in **39.85 ms** proxy round-trip.
After one fsynced 116-byte edit, publication-triggered refresh removed the old
passage in **53.78 ms**; a new foreground query issued after publication
returned the fresh passage in **21.80 ms** proxy round-trip. That passage was
then genuinely reopened; stale reopening was denied and the private clipboard
stayed unchanged.

These first source-built outcomes and sanitized reproduction recipes are
byte-verified under
`~/.local/share/maestria-release-evidence/ddf9bcb8/`, with archive checksums in
`SHA256SUMS`. They demonstrate a removable preview cost and a repaired native
path, **not** the cause of the earlier installed deadline, a successful
installed acceptance, or the historical `LockBusy` writer. PR #516 remains
draft; installed acceptance and genuine corpus/provider judgments are still
release gates.

### Bounded durable-audit and shutdown repairs (2026-10-01)

A disk-backed regression established a separate search deadline failure:
prepared durable access-audit persistence entered the main indexing lane and
waited for its occupied permit. Deferred persistence already bypassed that
lane. Prepared persistence now uses the same inline, watchdog-bounded path.
The regression requires the acknowledged query/trace audit to be readable
through an independent SQLite reopen **before** indexing is released; it
failed past **100 ms** before this repair.

Shutdown also returned early when the continuous-ingestion watcher failed,
without joining the runtime task. Both tasks are now joined, preserving and
combining their errors. A controlled pending-runtime regression failed before
the repair. A fresh private daemon smoke forced an actual watcher-state
persistence error: shutdown preserved the error, exited **1** in **65.27 ms**,
and required no additional signal or extension of the **5-second** gate.

With the source-built Arch daemon and unchanged installed `1d742124` Ubuntu
launcher/CLI, three fresh native Return/detail sequences exercised fresh
reopening, edited/deleted stale-action denial with unchanged clipboard, and
deferred no-match refresh without retyping. Five interactive replies had
independently reopened matching durable query/trace audits and no daemon error;
maximum proxy round-trip was **12.50 ms**. The driver subsequently failed in
its audit observer because it expected a nonexistent response wrapper. That
first failure is retained; the audit proof used retained replies, not replay.
Earlier root-namespace, consumer-status-schema, and incomplete native-settings
setup/observer failures are retained separately.

This is **not** fresh CI-installed acceptance or bilingual quality evidence.
It does not attribute the historical Return failures, Tantivy writer, or
original outer shutdown survivor. Historical writer attribution remains
unresolved and, by the bounded release decision, is **not an active release
gate**. Fresh exact-package installed acceptance and the frozen personal-corpus
French/English evaluation remain gates; PR #516 stays draft until they pass.
Authorization before content I/O, exact roots, fresh reopening, durable audit,
Bubblewrap isolation, cancellation, interactive-search admission, and the
**100-ms internal deadline** are unchanged.

### Disable, uninstall, and choose local-data retention

Before removing packages, stop their processes explicitly: close active
extension work and run `sillage-launcher --quit`; stop a foreground
`sillage-search start` with Ctrl-C. Package removal is not a process manager and
does not terminate an already-running application or daemon. If search access
should be revoked, do it while the daemon is running, then stop it:

```bash
INSTANCE="$HOME/sillage-search"
sillage-search owner grant list --instance-dir "$INSTANCE"
grant_digest="paste-the-grant-token-digest-printed-by-create-external"
sillage-search owner grant revoke --instance-dir "$INSTANCE" "$grant_digest"
sillage-search owner roots remove --instance-dir "$INSTANCE" "$HOME/Documents"
```

Remove only the component being disabled, or both packages for the combined
install:

```bash
sudo apt remove io-github-briolabs-sillage-launcher
sudo apt remove io-github-briolabs-sillage-search
sudo apt remove io-github-briolabs-sillage-extension-worker
# Or remove launcher and search together:
sudo apt remove \
  io-github-briolabs-sillage-launcher \
  io-github-briolabs-sillage-search
```

`apt remove` (and `apt purge`) removes system package files, not per-user data.
Keeping data requires no extra step. The default local paths are the selected
search instance (here `$HOME/sillage-search`), the credential file
`$HOME/.config/sillage/search-client.key`, launcher settings under
`${XDG_CONFIG_HOME:-$HOME/.config}/io.github.briolabs.Sillage.Launcher/`, and
extension bundles under
`${XDG_DATA_HOME:-$HOME/.local/share}/io.github.briolabs.Sillage.Launcher/extensions`.
Review custom instance, credential, and XDG paths before deleting anything.
After stopping processes and revoking any grants, remove only the local data
you intend to discard; these interactive commands prompt before deletion:

```bash
rm -ri -- "$HOME/sillage-search"
rm -i -- "$HOME/.config/sillage/search-client.key"
rm -i -- "${XDG_CONFIG_HOME:-$HOME/.config}/io.github.briolabs.Sillage.Launcher/launcher.toml"
rm -ri -- "${XDG_DATA_HOME:-$HOME/.local/share}/io.github.briolabs.Sillage.Launcher/extensions"
```

Deleting the search instance removes its local index and state, not source files
under the approved roots. The global-shortcut permission is owned by the user's
desktop portal, not by the Debian package; package removal does not revoke it.
Use the desktop's shortcut/application-permissions UI if the grant itself should
be revoked. When a later artifact for a component is available, install it in
place with `apt install ./path/to/new.deb` under the same package ID rather than
removing or purging first. The [exact-`59079579` Ubuntu 24.04 upgrade
job](https://github.com/brio-labs/maestria/actions/runs/36520546866/job/109252866382)
installed the independently built launcher, search, and extension-worker
`0.0.0` Debian artifacts, then upgraded each with `apt` to `0.0.1`.
The installed binaries preserved the private launcher settings, extension
permissions and data, search instance, root-scoped consumer grant and credential
through restart; the same state survived removal and reinstall without purge.
Authorized passage search and evidence reopen retained the original evidence
ID, ungranted roots remained excluded, and explicit grant revocations were
enforced. The job used private Xvfb/D-Bus, **not** a stock Wayland compositor;
it does not prove desktop portal-shortcut grant retention through a real
compositor upgrade. Check that grant in the desktop's shortcut/application
permissions UI after upgrading.

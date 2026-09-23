# Sillage Product Roadmap

This document is the single canonical product roadmap for Sillage. M1 has
a working native launcher slice but is not complete; M2–M4 remain planned.
Each milestone has explicit exit criteria. Historical retrieval reports do
not satisfy product-milestone evidence, and backend capabilities alone do
not count as launcher integrations.

## Milestone 1: Desktop Launcher

**Status:** In progress — native shell, catalog, commands, calculator, selected-file actions,
X11 shortcut and Wayland portal/fallback are implemented; daemon-backed file search is pending.

**Scope:** Build the resident Linux surface with a user-configurable global
shortcut and an explicit compositor-configured fallback; discover and launch
applications from XDG desktop entries; provide a host command registry, safe
arithmetic calculations, keyboard and focus behavior, open/copy actions, and
baseline latency instrumentation. The launcher uses the existing daemon for
file-search work but remains interactive when no daemon or model is available.

**Dependencies:** None. This milestone establishes the command and explicit
action contract consumed by later milestones.

**Exit criteria:** The product can be invoked, find an application, launch it,
reopen, calculate `2 + 2`, copy `4`, and dismiss without a daemon or model.
Wayland and X11 integration are verified separately, including the documented
fallback when a compositor cannot grant a global shortcut.

## Milestone 2: Extension Platform

**Status:** Planned

**Scope:** Deliver the Sillage-specific TypeScript SDK, versioned manifest
validation, development loading and local-bundle installation, host-rendered
declarative list/detail/form UI, capability grants, an authenticated broker,
isolated workers, disable/revoke behavior, validated atomic updates,
uninstall and extension-local data handling, resource bounds, cancellation,
crash handling, and author-facing examples. The initial platform excludes
Raycast, Node.js, native-addon, arbitrary-DOM, and React compatibility.

**Dependencies:** Depends on M1's command/action contract. SDK design and
broker/isolation work may proceed alongside M1. This milestone is mandatory,
not an optional post-launch ecosystem phase.

**Exit criteria:** An extension built using only the published SDK installs from
a local bundle, displays a searchable list/detail/form, and runs a permitted
action. Denied network/file access, cancellation, worker crash, and a
permission-expanding update behave as specified, with the previous working
version retained when validation or consent fails.

## Milestone 3: Semantic File Search

**Status:** Planned

**Scope:** Add approved read roots, live indexing and status, file-name/path and
lexical results, optional local dense enrichment, freshness handling, and
worker-level cancellation behind the M1 action contract. Reuse the daemon's
warm retrieval runtime, source hashes, authorization, and projection
generations; retain the last usable index during rebuilds and revalidate
availability and authorization before opening a result.

**Dependencies:** Depends on M1. It can progress independently of M2 behind
the same action contract.

**Exit criteria:** Exact path lookup works without a model; a held-out
paraphrase retrieves its relevant document with the configured provider; edits
and deletes become visible; out-of-scope documents never appear; and a stalled
provider leaves deterministic search responsive.

## Milestone 4: First Product Release

**Status:** Planned

**Scope:** Package the Linux product, complete permission and onboarding flows,
publish extension-author install/run instructions, and provide recovery
behavior plus complete latency, resource, and isolation evidence. The release
must keep current CLI daemon lifecycle behavior explicit while making desktop
startup an opt-in onboarding choice.

**Dependencies:** Depends on M1, M2, and M3.

**Exit criteria:** A fresh Linux user can install, invoke, launch, search,
install/use/remove an extension, and recover from a worker crash without CLI
repair. Publish the tested compositor matrix and benchmark environment;
unsupported global shortcuts use the explicit binding fallback.

## Existing reusable foundations

These capabilities exist in the current developer build and remain supported
backend or advanced surfaces. They are not wired into native launcher file search:

| Capability | Current status |
|---|---|
| CLI and authenticated per-instance daemon | Exists; lifecycle remains explicit |
| Approved read-root manifests and local indexing | Exists |
| Lexical search, evidence opening, and rebuildable projections | Exists |
| Warm retrieval runtime and typed retrieval/security boundaries | Exists |
| Dense semantic retrieval | Provider-dependent |
| Browser-hosted Studio and external ACP integration | Exists as a secondary surface; not an extension platform |
| Notebook, task, validation, approval, and memory workflows | Existing advanced capabilities |
| Repository/code and visual retrieval | Existing provider/freshness-degraded or research-only surfaces |

The resident Linux launcher, application catalog, host commands, calculations,
and platform shortcut integration exist. Daemon-backed file search, extension
lifecycle, TypeScript SDK, broker, isolated workers, and sandbox remain planned.
Retrieval benchmarks are not evidence that launcher file search shipped.

## Initial performance acceptance targets

These are **product acceptance budgets, not Raycast comparisons**. The launcher
slice has the limited X11 measurements below; the combined product, file search,
physical presentation, and background-indexing targets remain unverified:

- On a recorded Linux x86_64, SSD, at least 16 GiB RAM reference system, warm
  shortcut-to-interactive-window p95 is ≤100 ms; keystroke-to-app/command
  results p95 is ≤50 ms; warm filename/lexical first results p95 is ≤100 ms.
- For 10,000 eligible text files capped at 100 MiB total text and 500 desktop
  entries, optional local semantic first results p95 are ≤500 ms, and an exact
  file-path query ranks the matching authorized path first.
- Resident launcher plus daemon plus broker idle RSS is ≤200 MiB, excluding
  separately reported model-provider memory and active workers. Idle CPU
  averages ≤1% of one logical core over 60 seconds with no indexing.
- Measure cold startup separately, with a ≤1 second window-interactive target;
  model loading and index rebuild must not block it.
- Capture at least 200 warm interactions per latency class, with p50/p95/p99,
  hardware/session/provider/index identity, and a repeat while background
  indexing is active. Also report full-process-tree memory,
  indexing-throughput/disk-footprint, and energy where available; unavailable
  counters stay unavailable.
- Assess semantic quality using at least 50 held-out paraphrase queries with
  relevance judgments on the frozen corpus. Require recall@10 improvement over
  lexical search, no exact-path regression, and zero unauthorized exposure.
  Existing retrieval promotion requirements continue to apply; target budgets
  do not silently activate shadow routes.

### Launcher-only X11 measurement (2026-09-23)

On an Intel Core Ultra 7 258V, private Xvfb/JWM display, WebKit DPR 1,
Noto Sans, release build, and 500 frozen desktop entries, 200 warm activations
and 200 app queries produced:

| Observation | p50 / p95 / p99 |
| --- | --- |
| Native activation receipt to renderer-ready acknowledgment | 24.721 / 30.715 / 33.207 ms |
| Injected X11 shortcut dispatch to observed acknowledgment | 63.462 / 80.717 / 87.888 ms |
| Renderer query input to results-ready acknowledgment | 10 / 19 / 22 ms |
| Native query receipt to results-ready acknowledgment | 10.198 / 19.046 / 21.804 ms |

The renderer acknowledges after React commits and requests an animation frame;
this does **not** prove physical pixel presentation. Injected X11 keys include
tool and monitoring overhead and do **not** measure human physical-key latency.
A cold WebDriver-session-to-visible-interactive upper bound was **1,051.115 ms**:
it includes automation overhead, misses the ≤1 s bound as measured, and
neither proves nor disproves the application's standalone cold-start target.
Hidden idle CPU over 60.087 s was 0.1165% of one core. Launcher-only RSS was
180.434 MiB native plus 372.109 MiB WebKit subprocesses, totaling 552.543 MiB
in summed process RSS. The launcher alone exceeds 200 MiB on that measure;
shared-page accounting and the planned daemon/broker make this **not** a
certification of the combined-product memory budget. Raw local measurements
and caveats are in `target/launcher-evidence/launcher-measurements.json`.

## Non-blocking research and future scope

Advanced sparse, late-interaction, graph, temporal, counterevidence, fusion,
visual, and other retrieval research; AI agents; clipboard history; snippets;
cloud sync; a marketplace; and operating systems beyond the Linux target do
not block these four milestones. Their dated measurements remain research
evidence and must not be relabeled as launcher evidence.

The product roadmap advances only when its own milestone exit criteria are
shown. Existing retrieval reports, implementation status, provider adapters,
and issue order do not advance a product milestone or activate a shadow route.

# Sillage Product Roadmap

This is the single canonical product roadmap, not a report of shipped features.
The current developer build has a native **Slint** launcher, a locally built
search-only Debian package, and the existing CLI/daemon with document-content
retrieval. Legacy Tauri/React sources remain until behavioral parity and
release criteria permit their removal. Scoped external search and authorized
bounded, typed passage previews were exercised independently without a model.
An opt-in Slint client now displays cited passages and reopens evidence for
actions; an isolated X11 run covered content-only retrieval, copied reopened
text, changed-source denial and superseded app results. Ubuntu 24.04 apt
installation and installed-process search smoke passed locally; hosted CI,
Wayland passage actions and product release proof remain pending.
Neither this package nor local launcher packaging constitutes a product-milestone exit.
[PR #516](https://github.com/brio-labs/maestria/pull/516) remains a draft
baseline, not a release candidate.

## Product boundaries and work order

- **Content first:** find an answer-bearing passage *inside* an approved user
  file, show its bounded source excerpt and line/page, and offer a safe action to
  open or copy it. Filename/path search alone does not satisfy content search.
- **Take only what you need:** the launcher runs without a daemon or model;
  standalone content search runs without a launcher, Studio, extension broker,
  model, or AI answer generator. The combined product uses the same search
  engine and index owner, not a forked retrieval implementation.
- **Explicit consent:** approved read roots, per-consumer revocable access,
  source freshness, and authorization before retrieval, preview, and action.
  Embeddings and OCR are separate opt-in capabilities. No arbitrary typed shell
  evaluation, network listener, implicit indexing of the home directory, or
  unattended daemon/autostart installation.
- **Native experience:** replace Tauri with Slint in one clean cutover while
  preserving the existing binary, desktop/portal identity, user settings,
  shortcuts, and keyboard semantics. Build a distinctive, polished Sillage
  interface informed by Raycast's usability, not a copy of its assets.

M1 (Slint) and M2 (standalone search) can progress in parallel. M3 integrates
both and measures actual passage-finding utility. M4 (extensions) can progress
alongside M1–M3 once the command/action contract is stable. Only M5 requires
all four. **M2's search-only artifact can ship independently before M5; this
does not declare the combined product released.** The historical v0.9 research
milestone and [#60](https://github.com/brio-labs/maestria/issues/60) remain
separate and do not unblock product milestones.

### Execution order and review gate

| Wave | Unblocked work | Gate to the next wave |
|---|---|---|
| 0 | Fix [startup index ownership #517](https://github.com/brio-labs/maestria/issues/517), prove [Slint backend and license #518](https://github.com/brio-labs/maestria/issues/518), define [visual system #521](https://github.com/brio-labs/maestria/issues/521), and expose [standalone search boundary #524](https://github.com/brio-labs/maestria/issues/524). These are independent. | Actual behavior and licensing evidence; typed, scoped client contract. |
| 1 | Build [resident Slint shell #519](https://github.com/brio-labs/maestria/issues/519); in parallel, [root lifecycle #525](https://github.com/brio-labs/maestria/issues/525), [cited previews #526](https://github.com/brio-labs/maestria/issues/526), and [bounded queries #527](https://github.com/brio-labs/maestria/issues/527). | App-only shell still works; authorized lexical passages arrive without a model. |
| 2 | Port [native actions #520](https://github.com/brio-labs/maestria/issues/520) and [polished Slint UI #522](https://github.com/brio-labs/maestria/issues/522) independently after #519; add [document formats #528](https://github.com/brio-labs/maestria/issues/528). Start [extension SDK #535](https://github.com/brio-labs/maestria/issues/535) after the command contract is stable; evaluate [optional dense #533](https://github.com/brio-labs/maestria/issues/533) after #527. | Native parity, source coverage and judged quality, not mock screens or provider availability. |
| 3 | Complete [Slint cutover #523](https://github.com/brio-labs/maestria/issues/523), [search-only acceptance #529](https://github.com/brio-labs/maestria/issues/529), [launcher content #530](https://github.com/brio-labs/maestria/issues/530), and [extension isolation #536](https://github.com/brio-labs/maestria/issues/536). | Independently usable search and visible, authorized results in the real Slint UI. |
| 4 | Finish [source actions #531](https://github.com/brio-labs/maestria/issues/531), [ranking #532](https://github.com/brio-labs/maestria/issues/532), and [extension lifecycle #537](https://github.com/brio-labs/maestria/issues/537); then [passage-level acceptance #534](https://github.com/brio-labs/maestria/issues/534). | No exact-hit, freshness, or access regression. |
| 5 | Deliver [modular packaging #538](https://github.com/brio-labs/maestria/issues/538) and only then [combined release proof #539](https://github.com/brio-labs/maestria/issues/539). | Published product measurements and fresh-install evidence. |

Low-priority [#540–#544](https://github.com/brio-labs/maestria/issues/540)
remain independent, non-blocking options; their dependency links are in the
issues. This schedule follows actual prerequisites, not milestone numbers
alone. Every implementation follows [PHILOSOPHY.md](PHILOSOPHY.md):
deterministic kernel and acyclic dependencies; explicit governed effects,
typed domain/DTO/port boundaries, bounded channels and cancellation; source
snapshots and pre-score authorization; independently tested adapter contracts,
single-owner lifecycle, clean API cutovers, and measured per-query-class
retrieval promotion. No backend API handler mutates a database directly.

## Milestone 1: Slint Launcher and Native Experience

**Status:** In progress. The native Slint binary, X11/Wayland windows, and local
Debian/AppImage packages exist. Local X11 package smoke verified application
launch, arithmetic/clipboard, first-run deferral, Preferences, resident
reactivation, shortcut setup/restoration, Caps/Num-lock activation, and
conflicting-grab rejection. An isolated packaged Weston Wayland run with a
fake seat verified the first-run offer, AT-SPI deferral/focus, resident
reactivation, and quit. Ubuntu 24.04's Weston 13 lacks a fake seat; the
locally exercised no-seat package path verifies persistence, not focus.
The final packages were built against Ubuntu 24.04 and their executables
passed a `glibc` ≤2.39 ABI check. Ubuntu apt installation without search
reached a visible X11 window, but the container first-run AT-SPI offer check
timed out; X11 and nested Weston smoke passed on the host using these exact
Ubuntu-built Debian/AppImage payloads, not inside the Ubuntu container.
Hosted Ubuntu CI, live desktop Wayland portal/chooser acceptance, release
performance, and legacy-source removal remain open.
[Product milestone](https://github.com/brio-labs/maestria/milestone/10).

**Scope:** Decide Slint's distribution license and Linux X11/Wayland backend,
including software fallback and accessibility. Replace the resident Tauri
window, React/Vite renderer, plugin-owned shortcuts/dialogs/clipboard/opener,
WebDriver-only acceptance, and Tauri packaging with one Slint implementation.
Keep XDG application discovery, safe host commands, arithmetic, the selected-
file chooser, generation-aware query work, global shortcut or explicit
compositor fallback, and preferences. Define and implement an original visual
system: compact high-density results, typography, icons, clear grouping and
selection, action panel, keyboard hints, coherent loading/error/empty states,
HiDPI, light/dark contrast, reduced motion, and screen-reader semantics.

The [original visual reference](evidence/sillage-visual-reference.svg) is an
annotated design mockup; actual Slint X11/Wayland windows are documented in
[research evidence](RESEARCH.md#6-actual-slint-launcher-process-evidence-2026-09-24).

**Exit criteria:** On X11 and separately on Wayland, a fresh user can invoke,
find and launch an app, calculate `2 + 2`, copy `4`, dismiss and reopen without a
daemon or model. `--activate` routes to the existing resident process and
`--quit` exits cleanly. Existing `maestria-launcher`,
`io.github.briolabs.Maestria.Launcher`, portal grants, and `launcher.toml`
preferences survive the migration. Actual Slint-window screenshots and
keyboard/accessibility interactions demonstrate the approved visual design.
The old Tauri/React launcher, stale native tests, build dependencies, and
packaging are removed **only after** full behavioral parity is demonstrated;
Studio remains a separate web surface.

**Issues:** [Slint backend and license #518](https://github.com/brio-labs/maestria/issues/518)
→ [resident shell #519](https://github.com/brio-labs/maestria/issues/519)
→ [native actions and shortcuts #520](https://github.com/brio-labs/maestria/issues/520);
[visual system #521](https://github.com/brio-labs/maestria/issues/521) and
shell → [Slint UI #522](https://github.com/brio-labs/maestria/issues/522);
then [test, CI, and package cutover #523](https://github.com/brio-labs/maestria/issues/523).

## Milestone 2: Standalone Document-Content Search

**Status:** In progress. Bounded authenticated `sillage.search`, provider-owned
roots, durable watcher receipts, and external edit/delete/revoke/reapprove
denial/restoration were exercised across independent processes and restarts.
A separate search-only Debian artifact was built; its binary indexed an
explicit approved root, served independent grant-scoped search/evidence/status,
and denied an ungranted realm without launcher or model configuration.
Search-only grants receive byte-bounded, typed cited previews without
evidence-open permission; source edits and grant revocation deny stale content.
Ubuntu 24.04 container apt installation and installed-process smoke passed
locally for bounded v2 interactive Markdown, DOCX paragraph and text-PDF
citations, image-only PDF OCR-needed reporting, evidence reopen, restart,
revocation, and changed-source suppression. A fully indexed 10,000-file
uninstrumented warm exact-phrase probe returned 197/200 successful requests,
with successful separate-process CLI p95 78.42 ms and three daemon timeouts
at the unchanged 100 ms deadline. During approval/indexing of 700 more files,
the first six of 30 calls timed out with 429 initially pending; the later
24 succeeded as indexing reached zero pending. A later warm run on the
fully indexed retained corpus, after direct-path source lookup, succeeded
199/200 with successful CLI p95 51.20 ms; one daemon timeout remained.
With daemon tests running concurrently, 196/200 succeeded. The workloads
differ, so no causal latency gain or 100 ms acceptance is claimed.
Active-indexing and native whole-path latency have **not** passed. Hosted
CI, held-out passage relevance, portal approval/denial, extension isolation,
and combined release proof remain open. The v2 interactive API now returns
distinct authorized filename/path-only results for indexed text sources; it
does not invent a passage or line number, and skips PDF paths. An independent
search-only process found a filename-only source, suppressed an out-of-root
name, denied a deleted source immediately, and denied removed roots and
revoked grants. A 521-file settled private corpus initially produced four
cold-request timeouts, and three more after restart, at the unchanged 100 ms
daemon deadline. After query-first filename pruning, single-pass snapshot
replay, compact watcher-state persistence, and prewarming before socket
readiness, the rebuilt search-only daemon returned the late-position
filename-only match in 30/30 independent-process requests immediately after
restart (first CLI call 46.3 ms; CLI median 33.915 ms, maximum 69.0 ms).
The first request after a watched source edit still timed out; deleting the
source produced two timeouts before fresh queries returned no path. On a
separate private 521-file fixture, replaying all event families took 141 ms
during startup instrumentation; the indexed lexical-only source-event scan
took 18–41 ms across subsequent measured startup/update rebuilds. A first
changed-body query after indexing settled still timed out at the unchanged
100 ms deadline: one trace reached the source snapshot at 43 ms, the root
filter at 50 ms and the lexical result at 101 ms, before evidence reopen and
durable audit. The warm retry returned the changed cited passage.

The scoped interactive source filter now checks grant roots, manifest approval,
and privacy exclusions in one pass, caching the final filter by manifest and
exact grant roots rather than caching an all-roots filter then rescanning and
intersecting. A two-root A→B→A passage regression passed. In one local
source-built CLI run over 521 short Markdown files, the initial cold
interactive request still timed out (111.1 ms CLI elapsed). Once indexing
settled after a live edit, the first request returned the changed cited
passage in 66.7 ms CLI elapsed (warm retry 52.1 ms); the first request after
settled deletion denied it in 48.7 ms. This small-document, single-run result
does not establish the deadline on the earlier larger corpus or during active
indexing.

Instrumenting the same source-built cold-query path exposed a separate
readiness cost: after `indexing-status` reported 521 files settled, the request
spent about 42 ms building its source snapshot and 7 ms on the scoped filter,
then Tantivy's deferred writer commit took 111 ms and exceeded the deadline.
Committing each artifact before publishing its completion was tested and
reverted: indexing finished in 19.19 s, but the first query took 449.2 ms and
timed out. The existing batched writer behavior remains; a batch-level
visibility barrier is needed before considering the cold-query gate passed.

Two bounded commit thresholds were then measured and reverted on a fresh
521-short-Markdown-file standalone CLI fixture with an exact root grant.
Committing after 64 artifacts indexed in 18.28 s; the first settled cold
query (118.2 ms CLI) and first settled edit (112.9 ms) both timed out at the
100 ms daemon deadline. At 128 artifacts, indexing took 18.09 s, with
118.9 ms cold and 116.1 ms edit timeouts. A separate quiet-period writer
committer improved one settled run (113.1 ms cold and 106.2 ms edit CLI
elapsed, both returning cited passages) but timed out during active indexing
(118.4 ms at 128/521 accepted files) and on deletion in a second run
(119.4 ms). It was also reverted. Neither background timer nor threshold
establishes a correct completion/reader-visibility boundary or active-indexing
latency; both can leave multiple Tantivy segments for subsequent queries.

Startup prewarming and narrower replay do not establish active-indexing latency,
200-sample native whole-path performance or combined-release acceptance.
Version-18 grant storage now freezes a nonempty set of approved roots per new
consumer grant; old grants retain explicit legacy all-approved semantics until
revoked and reissued. A live two-sibling-root daemon regression exercised
root-specific passage and filename search, direct evidence denial, consumer
inventory, restart/replay, and root removal. With the rebuilt standalone CLI,
two independent consumers received different frozen root scopes: A returned
its cited passage and filename path but not B's, reported one indexed file,
and received typed `SourceNotSelected` for B's real evidence ID. The live
two-root integration also indexed a one-page PDF under B: B opened its
authentic page-1 excerpt and source path; A could neither search its passage
nor open its evidence ID (`SourceNotSelected`). After restart both scopes
persisted; removing A's approved root reduced A's inventory to zero while B
still retrieved its cited passage. A fresh Ubuntu 24.04 search-only Debian
package in `target/search-packages-current/` passed payload verification and
disposable apt-installed CLI/daemon smoke: approving B after issuing A's
credential did not expand A's scope, inventory or direct evidence access;
B's separate grant returned its cited passage and reopened evidence. Both
credentials persisted across restart. Fresh Ubuntu-built launcher Debian and
AppImage payloads passed metadata, desktop-ID and glibc-ABI verification; the
combined apt install and bounded installed X11 Slint smoke passed in disposable
Ubuntu 24.04. The test now enables `org.a11y.Status.IsEnabled` on its private
bus before launching Slint: without it, AccessKit produced a visible window
but no AT-SPI tree. The Debian and AppImage package smoke exercised the
first-run offer, deferral, Preferences, resident reactivation, calculations
and clipboard; X11 shortcut setup persisted across restart and rejected a
conflicting grab. Nested Weston exercised startup, offer/deferral, reactivation
and quit without verifying Wayland passage actions or live portal grants.
Active-indexing latency, hosted CI and combined-release acceptance remain
open; the original package directories still contain older artifacts.

[Product milestone](https://github.com/brio-labs/maestria/milestone/11).

**Scope:** Give another application a small, versioned authenticated local
search/evidence/status client and give the search engine a search-only service
and CLI lifecycle. Reuse existing parser, storage, governance, retrieval, and
source-version contracts; do not require the launcher, Studio, tasks, agents,
extensions, or an embedding provider. Let users approve/revoke roots and
inspect progress, exclusions, formats, index freshness, and OCR-needed state.
Return bounded, authorized excerpts and typed file lines/PDF pages, not just
evidence IDs or a formatted source string. Preserve exact source identity,
revalidate before opening, and reflect edits/deletes. Add an interactive
local-text plan with bounded, cancellable work: the general-purpose daemon
planner's 30-second allowance and uncancelled blocking tasks are not suitable
for every keystroke. Add safe DOCX extraction; OCR remains opt-in.

**Exit criteria:** A fresh search-only install, with no launcher or model,
indexes an approved directory and lets an independent client find an exact
phrase *inside* a text, Markdown, PDF, or supported DOCX document, inspect its
cited passage and open its source. Deleted, changed, revoked, unsupported, and
OCR-needed sources are represented honestly. A client without a scoped grant
cannot enumerate previews or open evidence. One index owner serves concurrent
consumers, with documented service restart and no public network listener.

**Issues:** [service/client boundary #524](https://github.com/brio-labs/maestria/issues/524)
→ [approved roots and freshness #525](https://github.com/brio-labs/maestria/issues/525),
[cited previews #526](https://github.com/brio-labs/maestria/issues/526), and
[bounded interactive search #527](https://github.com/brio-labs/maestria/issues/527);
[document formats and OCR state #528](https://github.com/brio-labs/maestria/issues/528)
→ [independent-client acceptance #529](https://github.com/brio-labs/maestria/issues/529).
Existing [Tantivy startup LockBusy #517](https://github.com/brio-labs/maestria/issues/517)
is a separate reliability blocker, not a reason to suppress the failure.

## Milestone 3: Integrated Content Discovery

**Status:** In progress. An optional separately authenticated search client
now shows grouped highlighted passages, typed citations, a document detail
view, filter categories over returned authorized evidence, and coverage/index
metadata in Slint. On isolated X11, a phrase inside an approved Markdown file
was visible, freshly reopened text copied, a changed source refused by the
old action, and the next app query replaced the passage. Nested Weston Wayland
showed a cited passage and detail, copied both excerpt and citation through
an isolated X11 clipboard fallback with fresh provider evidence-open audits,
and refused Open source after a source edit. A later private X11 Slint run
displayed a filename-only result in a separate File/Path row, copied its path
only after a fresh provider query, then refused to replace the clipboard after
deletion. A fresh private X11 run using Ubuntu-built launcher and search
executables byte-identical to the final Debian payloads again displayed
File/Path, copied the authorized path, showed an explicit denial after its
deletion, then displayed a highlighted body-only Markdown citation with
document detail and copied its freshly reopened excerpt. The actual native
clipboard helper also copied text in an isolated Weston compositor with
`DISPLAY` absent; an independent `wl-paste` read it through standard
Wayland data-device despite no data-control protocol. Weston's fake seat
lacks virtual-keyboard injection. A private AT-SPI session exposed the
pure-Wayland Slint window and focused search entry, but the entry did not
expose `EditableText` and synthesized keyboard text produced no result.

With the current source-built launcher/search pair in a disposable X11 session,
an AT-SPI-selected Markdown passage action showed the freshly reopened excerpt
and its line range in Slint, then invoked a private default viewer for the
approved source. A second genuine one-page PDF action showed its reopened
page citation and passed the verified `file:` URI with numeric `#page=1` to a
private PDF default handler. This proves the hint was dispatched, not that
arbitrary installed PDF viewers honor it; the Slint detail remains the exact
fallback. These were not current Ubuntu Debian payloads or pure-Wayland actions.

Complete native Slint pure-Wayland actions, per-consumer root filters, exact
viewer line/page jumps, native p50/p95/p99, held-out relevance,
active-indexing latency, and provider-backed paraphrase quality remain
unproved. M3 depends on M1's Slint command/action shell and M2's search-only
API, **not** on extensions.
[Product milestone](https://github.com/brio-labs/maestria/milestone/12).

**Scope:** Show applications/commands immediately, then searchable filename,
path, and *document passage* matches from the optional service. Group passages
by document; show highlighted excerpts, source line/page, scope, freshness and
explicit loading/unavailable states. Provide a focused content view, root/type
filters and exact source actions: reauthorize and reopen at the matching line
or page when supported; otherwise open the file and display/copy its location.
Add user-controlled alias/favorite/frecency ranking without logging document
content, while protecting exact matches and explicit commands. Optional local
multilingual dense retrieval enriches paraphrases after lexical results; no
provider may block or replace deterministic search. Keep experimental sparse,
visual, late-interaction and hierarchy/STAIR approaches benchmark-gated rather
than treating SOTA claims as a launcher promotion record.

**Exit criteria:** A query whose words occur only *inside* an approved file
finds a relevant excerpt in the Slint launcher; the user can inspect and open
or copy the precise evidence. No daemon still leaves apps and commands usable.
A held-out French/English paraphrase improves passage recall with an explicitly
configured provider, without regressing exact phrase/path matches or revealing
out-of-scope content. Edits/deletes, a provider stall, and rapid successive
queries cannot show actionable stale or cross-query passages. Measure the
whole path in the real native UI, not only the retrieval engine.

**Issues:** [grouped launcher passage results #530](https://github.com/brio-labs/maestria/issues/530)
→ [verified source actions #531](https://github.com/brio-labs/maestria/issues/531);
[alias/favorite/frecency #532](https://github.com/brio-labs/maestria/issues/532),
[optional semantic lane #533](https://github.com/brio-labs/maestria/issues/533)
→ [passage-level product evaluation #534](https://github.com/brio-labs/maestria/issues/534).

## Milestone 4: Extension Platform

**Status:** In progress; mandatory for the **combined** first product release,
not for the standalone search-only artifact. The TypeScript SDK/example,
versioned manifest, sealed local directory/ZIP store, explicit grant-diff
review, declarative native Slint views, capability broker, and bounded
bubblewrap/QuickJS worker are implemented in the developer build. An isolated
X11 session installed the example through native consent, displayed a freshly
cited search excerpt under a separate owner-issued external provider grant,
and copied its greeting through the actual X11 clipboard. Denying an update
that added notification permission kept the earlier sealed package active;
a deliberate worker exception surfaced as an error, closing a running
infinite-loop command cancelled its worker, and revocation blocked commands
despite the independent provider grant. Native removal retained or deleted
private data as selected. The manager opened without the worker sibling.
The independently built Ubuntu 24.04 worker Debian apt-installed with
`bubblewrap` in a disposable Ubuntu container, and the Ubuntu-built worker
ran there. Disposable Ubuntu 24.04 apt installs also confirmed independent
launcher-only, search-only, and combined package selections, with no worker
pulled in by the combined install; worker installation is separately opt-in.
An installed search CLI initialized an approved root, issued a private
consumer credential, and stopped its foreground daemon on SIGINT. The
launcher/search Debian payloads were rebuilt on Ubuntu 24.04 with the new
filename/path code; the launcher declares `wl-clipboard` for pure Wayland.
A fresh combined apt install ran the installed search daemon and CLI, found
one filename-only authorized source without invented passages, denied it
after deletion, then stopped the daemon and removed search while retaining
the launcher and user instance. Host X11 then ran launcher/search executables
byte-identical to those Debian payloads: Slint rendered a filename-only row
and a cited body-only Markdown passage, copied authorized path and reopened
excerpt, and explicitly denied Copy Path after deletion. This does not prove
an Ubuntu-container native UI, extension-enabled packaged UI, or combined
release. Hosted CI, full pure-Wayland Slint clipboard actions, live portal
decisions, cross-compositor accessibility/isolation acceptance, active-indexing
latency, and combined release proof remain open. The repository-wide
philosophy gate still reports non-extension module, function-size, and parser
violations; extension-specific findings were addressed locally.

The launcher Debian verifier now rejects declared relationships that force or
prevent co-installation with the optional search/worker packages. A previously
built Ubuntu launcher Debian/AppImage passed the new verifier in a disposable
Ubuntu CI image. The updated launcher-only native package smoke also checks
that search and worker are uninstalled and constrains all launcher invocations
to `/usr/bin`; it has not yet been run against freshly rebuilt packages. This
metadata proof is not hosted CI or installed-extension UI acceptance.

[Product milestone](https://github.com/brio-labs/maestria/milestone/13).

**Scope:** Publish the Sillage-specific TypeScript SDK and versioned manifest,
host-rendered declarative list/detail/form UI, an authenticated capability
broker, isolated bounded workers, explicit user grants, local development and
bundle installation, safe atomic update/rollback, disable/revoke/uninstall,
and author examples. No Raycast, Node.js, native-addon, arbitrary DOM, or React
compatibility is promised.

**Exit criteria:** A developer uses only the published SDK to install an
extension, show a searchable view and run an authorized action. File/network
access denied by policy stays denied; cancellation, worker crash, permission-
expanding update, and uninstall behave as specified. A failed or unconsented
update preserves the previous working version.

**Issues:** [SDK and declarative UI #535](https://github.com/brio-labs/maestria/issues/535)
→ [broker and workers #536](https://github.com/brio-labs/maestria/issues/536)
→ [safe bundle lifecycle #537](https://github.com/brio-labs/maestria/issues/537).

## Milestone 5: First Combined Product Release

**Status:** In progress locally. Ubuntu 24.04 apt installation accepted
launcher-only, search-only, combined, and separately opted-in worker packages;
the refreshed combined launcher/search Debian install served a filename-only
approved result through the installed independent client, denied the deleted
source, and removed search while leaving the launcher and instance intact.
On isolated host X11, the byte-identical final Debian executable payloads
displayed an approved File/Path row and a highlighted body-only cited passage
in Slint; Copy Path and Copy Passage wrote freshly authorized content to the
private clipboard, while a deleted path action was explicitly refused.
`docs/OPERATIONS.md` documents root consent, grants, explicit daemon
lifecycle, disable/uninstall, and user-data choices. A same-version settings
reinstall passed, but real upgrades/shortcut-portal preservation,
Ubuntu-container native UI, hosted CI, full compositor and accessibility
matrix, and shipped-artifact latency/resource acceptance remain unproved.
Depends on M1, M2, M3 and M4.
[Product milestone](https://github.com/brio-labs/maestria/milestone/14).

**Scope:** Package launcher-only, search-only and combined installations with
consent-based onboarding, recovery, component removal, permission and data
lifecycle, extension-author instructions, a tested compositor matrix and
honest product-level latency/resource/isolation evidence. Installation does
not silently start a daemon, model, OCR provider, or desktop autostart. Keep
CLI daemon lifecycle explicit.

**Exit criteria:** A fresh Linux user can choose only the part they need:
launch applications with no search service; independently search and open cited
content with no launcher; or combine both and install/use/remove an extension
without CLI repair. X11 and Wayland behaviors, unsupported compositor shortcut
fallback, package identity, licensing/attribution, accessibility, resource
budgets and crash recovery are verified on shipped artifacts.

**Issues:** [modular packaging and onboarding #538](https://github.com/brio-labs/maestria/issues/538)
→ [product exit evidence #539](https://github.com/brio-labs/maestria/issues/539).

## Existing foundations and developer-build integrations (not a release claim)

| Capability | Current status |
|---|---|
| CLI and authenticated per-instance daemon | Exists; lifecycle explicit, protocol embedded in daemon crate |
| Approved read roots, supported local text/PDF ingestion, watcher | Search-only Debian and developer build with explicit owner root approval and documented daemon lifecycle; v18 per-consumer frozen root grants passed local A/B, restart, removal, and PDF-denial regressions, but current package and release latency gates remain open |
| Lexical passage retrieval and evidence opening | Exists in the standalone CLI/daemon; an optional credential-path Slint client shows cited results and reopens actions without starting the indexer |
| `maestria-retrieval` traits, typed plans, authorization, generations | Exists; daemon assembles the production engine |
| Dense semantic retrieval | Provider-dependent, measured for selected daemon query classes only |
| Native Slint launcher UI | Built and locally native-smoked on X11/Wayland and from Debian/AppImage; first-run deferral persisted across packaged X11 runs; not a release-certified product |
| Document-result UI and standalone search install | Search-only Debian independently smoked for Markdown/DOCX/PDF on Ubuntu 24.04; combined apt install found a filename-only source and denied its deletion. Final Ubuntu-built launcher/search executables byte-identical to Debian payloads rendered native X11 File/Path and body-only cited passage rows, copied authorized path and reopened excerpt, and explicitly denied a deleted path action. Nested Weston passage action used a private X11 clipboard fallback; the native helper separately copied/denied over-limit text in pure Weston Wayland without `DISPLAY`. Full pure-Wayland Slint action and certified combined release remain open |
| Browser-hosted Studio and external ACP | Existing secondary surface, not launcher UI or extension SDK |

The current Slint launcher discovers XDG applications, offers host commands,
calculates arithmetic, opens a file manually selected in a chooser, and can
optionally query a separately installed authenticated search client for bounded
typed passage previews. File/DOCX/PDF source actions re-open evidence under the
current grant; PDF opens use a typed current-source path, while default viewers
may not jump to the cited line/page. Ordinary images still need OCR and do not
fabricate text. Do not relabel existing daemon retrieval scores as launcher
quality or claim the combined product is release-certified.

## Initial performance and quality acceptance targets

These are **product budgets, not Raycast comparisons**. Existing latency
percentiles below were measured on Tauri, not on the Slint release build.
Re-measure actual window presentation
and the search-only service independently as well as the combined install:

- On a recorded Linux x86_64, SSD, at least 16 GiB RAM reference system, warm
  shortcut-to-interactive-window p95 is ≤100 ms; keystroke-to-app/command
  results p95 is ≤50 ms; warm first *lexical content passage* results p95 is
  ≤100 ms. Report source-preview rendering separately from index lookup.
- For 10,000 eligible text files capped at 100 MiB total text and 500 desktop
  entries, optional local semantic first results p95 are ≤500 ms; exact
  authorized path and phrase matches keep their first relevant file/passage.
  PDF and DOCX extraction, OCR, initial indexing and reindexing have separate
  named reference corpora and timings; never disguise ingestion as query time.
- Resident launcher plus daemon plus broker idle RSS is ≤200 MiB, excluding
  separately reported model-provider memory and active workers. Also report
  launcher-only and search-only RSS. Idle CPU averages ≤1% of one logical core
  over 60 seconds with no indexing.
- Measure cold startup separately, with a ≤1 second window-interactive target;
  model loading and index rebuild must not block it.
- Capture at least 200 warm interactions per latency class, with p50/p95/p99,
  hardware/session/provider/index identity, and a repeat while background
  indexing is active. Also report full-process-tree memory,
  indexing-throughput/disk-footprint, and energy where available; unavailable
  counters stay unavailable.
- Judge exact phrases, paths, and at least 50 held-out French/English
  paraphrases against **document- and passage-level** relevance on a frozen
  personal-content corpus. Require semantic passage recall@10 improvement over
  lexical search, no exact phrase/path or first-hit regression, correct
  preview-to-source navigation, and zero unauthorized exposure. Existing
  retrieval promotion requirements still apply; target budgets do not
  silently activate shadow routes.

### Historical Tauri launcher-only X11 measurement (2026-09-23)

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
These measurements remain historical after migration; they are not Slint
performance evidence.

Dated Slint backend and license feasibility evidence lives in the
[research note](RESEARCH.md#5-slint-backend-feasibility-evidence-2026-09-23).

## Non-blocking product scope and research

[Quicklinks #540](https://github.com/brio-labs/maestria/issues/540),
[snippets #541](https://github.com/brio-labs/maestria/issues/541),
[privacy-preserving clipboard history #542](https://github.com/brio-labs/maestria/issues/542),
[Linux window management #543](https://github.com/brio-labs/maestria/issues/543), and
[optional cited answers #544](https://github.com/brio-labs/maestria/issues/544)
are separate, low-priority modules, not first-release dependencies. Cloud sync,
a marketplace, other operating systems, and advanced sparse/late-interaction,
graph, temporal, counterevidence, fusion and visual RAG research likewise do
not block M1–M5. Structured section metadata is worth evaluating for long
personal documents; neither STAIR fine-tuning nor text-image retrieval is a
launcher default without a dedicated judged corpus.

Product milestones advance only when their own exit criteria are demonstrated.
An issue number, a provider adapter, a retrieved benchmark, or a draft PR does
not constitute shipped functionality or activate an unpromoted retrieval route.

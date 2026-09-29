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
- Measurement evidence (benchmark reports) is recorded in
  `tests/contracts/benchmark_evidence_v1.json` and validated in CI. It
  preserves historical retrieval measurements and does not certify completion
  of the new product milestones.
- Lint-exemption expiries in `scripts/philosophy_check` are calendar dates
  (`YYYY-MM-DD`), enforced by `philosophy-check` on every run.

### Desktop launcher lifecycle

The launcher package installs a desktop entry for the user to invoke; installing
it does not launch the application or enable login autostart. The entry runs
`maestria-launcher --activate`. Shortcut setup is user-initiated. The search
daemon is a separate foreground process and remains explicitly started and
stopped by its operator.


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

## 8. Opt-in Linux package onboarding

Debian artifacts built from this source revision use version `0.0.1` and
architecture `amd64`; CI builds them on Ubuntu 24.04. From the repository root,
install the launcher, headless search service, or both with `apt` so Ubuntu
resolves the declared runtime dependencies:

```bash
# Apps only: no search daemon or worker is installed.
sudo apt install ./target/launcher-packages/maestria-launcher_0.0.1_amd64.deb

# Search only: no launcher or worker is installed.
sudo apt install ./target/search-packages/maestria-search_0.0.1_amd64.deb

# Combined: explicitly select both components.
sudo apt install \
  ./target/launcher-packages/maestria-launcher_0.0.1_amd64.deb \
  ./target/search-packages/maestria-search_0.0.1_amd64.deb
```

The Debian package IDs are `io-github-briolabs-maestria-launcher` and
`io-github-briolabs-maestria-search`; the launcher binary/desktop identity
remain `maestria-launcher` and `io.github.briolabs.Maestria.Launcher`.
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
`io.github.briolabs.Maestria.Launcher` identity and `activate-launcher` /
`CTRL+space`. Approval returned the matching Bind response `0` with
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
add a **compositor-owned** shortcut for `maestria-launcher --activate` in your
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
maestria-search init --instance-dir "$INSTANCE" --read-root "$READ_ROOT"
```

This stores the instance, index, credentials and watcher state under
`$INSTANCE`; the original files under `READ_ROOT` are not moved. To add or
remove a root later, keep the daemon running and use its owner commands:

```bash
maestria-search owner roots status --instance-dir "$INSTANCE"
maestria-search owner roots add --instance-dir "$INSTANCE" /absolute/reviewed/path
maestria-search owner roots remove --instance-dir "$INSTANCE" /absolute/reviewed/path
```

`add` and initialization reject a path that is not an existing directory or is
a symlink. Starting the daemon begins continuous indexing only for its approved
roots. Review the reported roots, exclusions and indexing freshness before
leaving it active. Hidden paths, symlinks, ignore files, and the built-in
privacy exclusions are skipped; these defaults do not replace choosing narrow
roots and checking the status output.

Run the daemon in a terminal where its lifecycle is visible:

```bash
maestria-search start --instance-dir "$INSTANCE"
# Stop it in this terminal with Ctrl-C.
```

`maestria-search start` uses the read-only profile and no model client. There
is no `maestria-search stop` command or package-installed service manager:
installation and launcher startup do not start it in the background. Root
changes apply while the daemon runs; after stopping it, restart explicitly.

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

### Create a scoped credential and configure launcher search

For direct headless search, or for the launcher to display passages, issue an
external grant while the provider daemon is running. Use the narrower
`search-only` access for a client that only needs bounded search previews; the
launcher passage flow also reopens evidence and therefore needs
`search-and-open-evidence`. The example below uses that launcher access and
caps it at five internal results with 4 KiB evidence and a one-day grant:

```bash
INSTANCE="$HOME/sillage-search"
CREDENTIAL="$HOME/.config/sillage/search-client.key"
(umask 077; mkdir -p "$(dirname "$CREDENTIAL")")
consumer_realm="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
maestria-search owner grant create-external --instance-dir "$INSTANCE" \
  --consumer-realm "$consumer_realm" --credential-file "$CREDENTIAL" \
  --access search-and-open-evidence --max-sensitivity internal \
  --read-root "$READ_ROOT" \
  --max-results 5 --max-evidence-bytes 4096 --expires-in-seconds 86400
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

Pre-v18 grants retain legacy **all currently approved roots** semantics,
including roots added later. `owner grant list` labels them
`allowed_roots=legacy-all-approved`; newly issued grants print the frozen root
list. Revoke and reissue any legacy grant before adding a root its consumer
must not read. Root scopes cannot be edited on an issued credential.

The command prints the grant digest and consumer realm, but not the bearer
credential; it creates the credential file with owner-only mode `0600`. Save
the printed grant digest for later revocation. For a headless client, give
`maestria-search search` the provider socket, this realm and credential file:

```bash
maestria-search search \
  --socket-path "$INSTANCE/system/daemon.sock" \
  --consumer-realm "$consumer_realm" --credential-file "$CREDENTIAL" \
  --limit 5 "a phrase from an approved source"
```

To opt the launcher into passages, edit its user-owned schema-1 settings file at
`${XDG_CONFIG_HOME:-$HOME/.config}/io.github.briolabs.Maestria.Launcher/launcher.toml`.
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
Keep `maestria-search` installed and on `PATH`. Restart the launcher after
editing (`maestria-launcher --quit`, then `maestria-launcher --activate`) so it
loads the new table. Removing the optional `[search]` table and restarting
disables launcher passage requests without uninstalling either package; revoke
the provider grant separately if access should end for every client.

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
sudo apt install ./target/extension-packages/maestria-extension-worker_0.0.1_amd64.deb
```

Its package ID is `io-github-briolabs-maestria-extension-worker`; it brings
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

### Disable, uninstall, and choose local-data retention

Before removing packages, stop their processes explicitly: close active
extension work and run `maestria-launcher --quit`; stop a foreground
`maestria-search start` with Ctrl-C. Package removal is not a process manager and
does not terminate an already-running application or daemon. If search access
should be revoked, do it while the daemon is running, then stop it:

```bash
INSTANCE="$HOME/sillage-search"
maestria-search owner grant list --instance-dir "$INSTANCE"
grant_digest="paste-the-grant-token-digest-printed-by-create-external"
maestria-search owner grant revoke --instance-dir "$INSTANCE" "$grant_digest"
maestria-search owner roots remove --instance-dir "$INSTANCE" "$HOME/Documents"
```

Remove only the component being disabled, or both packages for the combined
install:

```bash
sudo apt remove io-github-briolabs-maestria-launcher
sudo apt remove io-github-briolabs-maestria-search
sudo apt remove io-github-briolabs-maestria-extension-worker
# Or remove launcher and search together:
sudo apt remove \
  io-github-briolabs-maestria-launcher \
  io-github-briolabs-maestria-search
```

`apt remove` (and `apt purge`) removes system package files, not per-user data.
Keeping data requires no extra step. The default local paths are the selected
search instance (here `$HOME/sillage-search`), the credential file
`$HOME/.config/sillage/search-client.key`, launcher settings under
`${XDG_CONFIG_HOME:-$HOME/.config}/io.github.briolabs.Maestria.Launcher/`, and
extension bundles under
`${XDG_DATA_HOME:-$HOME/.local/share}/io.github.briolabs.Maestria.Launcher/extensions`.
Review custom instance, credential, and XDG paths before deleting anything.
After stopping processes and revoking any grants, remove only the local data
you intend to discard; these interactive commands prompt before deletion:

```bash
rm -ri -- "$HOME/sillage-search"
rm -i -- "$HOME/.config/sillage/search-client.key"
rm -i -- "${XDG_CONFIG_HOME:-$HOME/.config}/io.github.briolabs.Maestria.Launcher/launcher.toml"
rm -ri -- "${XDG_DATA_HOME:-$HOME/.local/share}/io.github.briolabs.Maestria.Launcher/extensions"
```

Deleting the search instance removes its local index and state, not source files
under the approved roots. The global-shortcut permission is owned by the user's
desktop portal, not by the Debian package; package removal does not revoke it.
Use the desktop's shortcut/application-permissions UI if the grant itself should
be revoked. When a later artifact for a component is available, install it in
place with `apt install ./path/to/new.deb` under the same package ID rather than
removing or purging first. A same-version Ubuntu apt reinstall preserved an
existing schema-1 `launcher.toml`, but this does not establish behavior across a
version upgrade or prove live desktop portal-grant persistence; check grants in
the desktop UI after an actual upgrade.

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

The current observer uses a one-second polling interval and the runtime's
bounded channel for backpressure. To pause continuous ingestion, stop the
daemon (`Ctrl-C` or the service manager); restart it after changing the
manifest roots or exclusions. There is intentionally no hidden background
process or network watcher. Removed paths are retained in the watch-state
tombstone map for explicit operational review rather than being silently
forgotten.

## 6. Versioning Posture

Sillage is in continuous development with no external release promise.

- The workspace version is pinned at `0.0.0`; `main` is always the current
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

The current local Debian artifacts are version `0.0.0`, `amd64`, and built on
Ubuntu 24.04. From the repository root, install the launcher, headless search
service, or both with `apt` so Ubuntu resolves the declared runtime dependencies:

```bash
# Apps only: no search daemon or worker is installed.
sudo apt install ./target/launcher-packages/maestria-launcher_0.0.0_amd64.deb

# Search only: no launcher or worker is installed.
sudo apt install ./target/search-packages/maestria-search_0.0.0_amd64.deb

# Combined: explicitly select both components.
sudo apt install \
  ./target/launcher-packages/maestria-launcher_0.0.0_amd64.deb \
  ./target/search-packages/maestria-search_0.0.0_amd64.deb
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
The current Ubuntu Debian and AppImage passed this bounded local smoke,
including first-run offer/deferral and shortcut persistence. This does not
verify live desktop portal approval, Wayland passage actions or a version
upgrade.

The launcher-only package smoke requires the launcher Debian to be installed
at the package version under inspection, checks that neither the separate
search nor extension-worker package or executable is installed, and uses only
system paths for all launcher interactions. Run it on a clean launcher-only
Ubuntu installation; the combined-install smoke has a different purpose.
The new absence/PATH assertions have not yet been exercised in hosted CI.
The Debian verifier independently checks that no launcher package relationship
pulls in, conflicts with, or claims the separately optional components.

On a pure Wayland session with no `DISPLAY`, the launcher uses the
`wl-clipboard` runtime dependency to copy text through the standard Wayland
clipboard protocol; its short-lived `wl-copy` parent exits after establishing
clipboard ownership. Text over 64 KiB is rejected before launching `wl-copy`;
nonblocking pipe writes and parent acceptance have one 500 ms post-spawn
deadline and report errors on timeout. The package still does not start a
search daemon.

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
sudo apt install ./target/extension-packages/maestria-extension-worker_0.0.0_amd64.deb
```

Its package ID is `io-github-briolabs-maestria-extension-worker`; it brings
`bubblewrap` for the launcher's OS sandbox. Installing it alone does not install
an extension, grant extension capabilities, start a daemon, or execute extension
code. The launcher refuses to run extensions outside the sandbox if the worker
or sandbox setup is missing.

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

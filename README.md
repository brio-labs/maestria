# Sillage

Sillage is an open-source, Linux-first keyboard launcher, independently usable
document-content retrieval component, and extension platform.

That is the target product direction. The current developer build provides a
native **Slint** launcher, a separately built headless search-only Debian
package, and the existing CLI/daemon/Studio. With an explicit authenticated
search configuration, the launcher can display bounded source-backed passages
without embedding the indexer or starting a model; app search still works
without that configuration or service. The extension SDK and complete product
release proof remain open.
See [the product roadmap](docs/ROADMAP.md) for the work and GitHub issues.

## Product direction

The primary launcher loop is: invoke → type → inspect an application, command
or **source-backed passage inside an approved file** → perform an explicit
action → dismiss. Content results must show the actual excerpt and line/page,
not merely a matching filename. The current Slint launcher discovers installed
applications, exposes registered host commands, performs safe arithmetic,
opens or copies files selected in the native chooser, and optionally groups
cited Markdown/DOCX/PDF passages from the separately installed search service.
Passage actions reopen evidence under the grant before copying or opening a
validated source; the default viewer may not jump to its cited line or page.
Embeddings and OCR are optional; app launch and exact/lexical search do not
require a model.
“Commands” means registered host or extension actions, not arbitrary shell
evaluation of typed text.

The current build and target product remain intentionally different surfaces:

| Capability | Status |
|---|---|
| File-content indexing, lexical passage search and evidence opening | Available in the separate CLI/daemon; optionally displayed as grouped cited passages in the native launcher after an explicit credential-path configuration. No daemon or model starts with the launcher |
| Dense semantic search | Provider-dependent in the CLI/daemon workflow; not a launcher default |
| Native resident launcher and application catalog | Available in the Slint developer build on X11 and Wayland; Debian and AppImage packages built and native-smoked |
| Distinctive interface and full Tauri-to-Slint parity | Slint search, Preferences, About attribution, theme and keyboard basics work; parity and release performance measurements remain open |
| Search-only installable service and scoped third-party client | `maestria-search` is built as a separate headless Debian package; its local Unix-socket API and external-process search/evidence commands work without launcher or model configuration. Search-only grants receive bounded cited previews without evidence-open permission. Ubuntu 24.04 container apt installation and installed-binary smoke passed locally; hosted CI and full product acceptance remain unverified |
| X11 global shortcut | Available after user setup in Preferences |
| Wayland global shortcut | Portal where supported; otherwise bind `maestria-launcher --activate` in the compositor |
| Extension lifecycle, SDK, isolated workers and capability broker | Planned |

The existing CLI and daemon quick start below is for the current developer
build, not launcher onboarding. Notebook, task, and memory workflows remain
supported advanced existing capabilities.

## Install

Sillage targets Rust stable 1.95+. Build from source:

```bash
git clone https://github.com/brio-labs/maestria.git
cd maestria

# Build the CLI binary
cargo build --release -p maestria-cli
./target/release/maestria-cli --help
```

Sillage has no releases: the workspace version is pinned at `0.0.0` and
`main` is always the current build. Build the CLI and daemon from source.

### Native Linux launcher

The current developer launcher uses Slint's software renderer and does not need
Node, pnpm, GTK3, or WebKitGTK. Build it with Rust stable 1.95+ and the native
X11/Wayland development libraries used by the
[CI dependency setup](.github/actions/setup-system-dependencies/action.yml):

```bash
cargo build --release -p maestria-launcher
./target/release/maestria-launcher --activate
```

To build the Debian and AppImage packages, install the pinned Cargo Packager:

```bash
cargo install cargo-packager --locked --version 0.11.8
NO_STRIP=1 APPIMAGE_EXTRACT_AND_RUN=1 cargo packager --release --packages maestria-launcher --formats deb,appimage
```

The packages land under `target/launcher-packages/`. Build distributable
artifacts on the oldest supported runtime, such as an Ubuntu 24.04 builder:
packaging a binary compiled on this Arch host required `GLIBC_2.43` and the
apt-installed launcher could not start on Ubuntu 24.04 (`glibc` 2.39).
`NO_STRIP=1` avoids an incompatible bundled linuxdeploy `strip` on systems
whose ELF libraries use `.relr.dyn`; it does not change runtime compatibility.
`APPIMAGE_EXTRACT_AND_RUN=1` lets linuxdeploy's AppImage plugin run without
FUSE inside a container; the builder also needs the `file` utility. A Debian
installation provides `maestria-launcher` on `PATH` and a desktop entry whose
`Exec` is
`maestria-launcher --activate`. No daemon or model is started, and installation
does not enable autostart. Use `maestria-launcher --activate`
to show and focus the resident window and `maestria-launcher --quit` for an
explicit shutdown. Closing or unfocusing the window hides it without ending
the resident process.

The final Debian and AppImage were built against Ubuntu 24.04, and both
packaged executables passed a `glibc` ≤2.39 ABI check and native X11/Wayland
smoke on this host. The Debian apt-installed into a disposable Ubuntu 24.04
container with no search service and opened a visible X11 window, but that
container's first-run AT-SPI offer assertion timed out. The host package UI
smoke does not establish Ubuntu container accessibility acceptance.

On first run, the launcher offers shortcut setup without blocking application
search. “Not Now” persists a deferred choice; Preferences remains available
with Ctrl+Comma for later setup. No global keybinding is installed until the
user requests it. Local Debian/AppImage X11 package smoke exercised first-run
deferral, arithmetic-result copying through the clipboard, Preferences, and
resident reactivation. An isolated Debian-binary run also launched an XDG
desktop-entry fixture and verified successful shortcut setup across restart,
Caps/Num-lock activation, and rejection of a conflicting grab without changing
the saved shortcut. A packaged Weston Wayland run with a fake seat verified
the first-run offer, AT-SPI deferral and search focus, resident reactivation,
and clean quit. Ubuntu 24.04's Weston 13 has no fake seat; the locally forced
no-seat package smoke confirmed deferral persistence and reactivation but cannot
test keyboard focus. Hosted Ubuntu CI and live desktop portal approval/denial,
chooser, and full screen-reader interactions remain unverified.

Open Preferences with Ctrl+Comma; the shortcut editor receives keyboard focus.
Set up the X11 shortcut there; the initial suggestion is
`Control+Space`. On Wayland, the global-shortcuts portal needs a desktop entry
with the matching `io.github.briolabs.Maestria.Launcher` identity and a
resolvable `Exec`. The Debian package provides it; an AppImage user must
integrate its desktop entry with a valid installed executable before portal
setup. The compositor owns the actual key combination. If it cannot assign
a portal shortcut, bind a user-chosen key to `maestria-launcher --activate`
(or to the user's chosen AppImage executable with `--activate`). For a
Hyprland configuration using its Lua API, a portal binding can use
`hl.bind("CTRL + SUPER + F12", hl.dsp.global("io.github.briolabs.Maestria.Launcher:activate-launcher"))`;
this is a **user-controlled example**, not a shipped or enabled default.
Tiling compositors can enlarge the window; configure a user-owned floating
and size rule if desired. File selection uses a native chooser and does not
index the selected directory.

The existing `maestria-launcher` executable, Debian package ID
`io-github-briolabs-maestria-launcher`, and
`io.github.briolabs.Maestria.Launcher` desktop/portal identity remain stable
under the Sillage name. Existing preferences, shortcut grants, and compositor
bindings continue to work without migrating or resetting user data.

To opt into document results, install `maestria-search` separately and create
an approved root plus a `search-and-open-evidence` consumer grant as shown
below. After launching once to create `launcher.toml`, add this table to
`$XDG_CONFIG_HOME/io.github.briolabs.Maestria.Launcher/launcher.toml` (or
`~/.config/io.github.briolabs.Maestria.Launcher/launcher.toml` without
`XDG_CONFIG_HOME`), replacing the paths and realm with the actual absolute
socket path, 64-character hexadecimal consumer realm and mode-0600 credential
file. Keep `maestria-search` on `PATH`; the launcher invokes it as a separate
bounded process and never reads the credential contents into its UI:

```toml
[search]
socketPath = "/home/you/Documents/sillage-search/system/daemon.sock"
consumerRealm = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
credentialFile = "/home/you/.config/sillage/search-client.key"
```

Without this table, without a running service, or after a denial, application
search remains available. A local isolated X11 Slint run found a phrase only
inside an approved Markdown file, displayed its highlighted citation and full
passage, copied freshly reopened evidence, refused the old copy action after
the source changed, and showed an app result after a newer query. This is not
a Wayland passage-action, live portal, whole-path latency, or release test.

In a historical **Tauri-only** X11 run with 500 frozen desktop entries and 200 samples per
class, native activation receipt to renderer-ready p95 was 30.715 ms and query
input to results-ready p95 was 19 ms. These acknowledgments are not physical
pixel presentation. A cold WebDriver-inclusive upper bound of 1,051.115 ms
missed the ≤1 s measurement target; it does not isolate app startup. Summed
launcher and WebKit RSS was 552.543 MiB, above 200 MiB without a daemon or
broker. Neither result certifies the planned combined-product targets. The
hardware, p50/p95/p99, idle CPU, and limitations are in [the roadmap](docs/ROADMAP.md).

## Current developer build quick start

The commands below exercise the current CLI and daemon; they are not
launcher-onboarding commands.

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

# 10b) Manage the learned-sparse promotion record
maestria promotion show -i .maestria-dev
maestria promotion set -i .maestria-dev --record learned_sparse_promotion_v1.json
maestria promotion remove -i .maestria-dev

# 11) Start the daemon (or restart after changes)
maestria start -i .maestria-dev
# Governance profile: read-only (default) or trusted-workspace; the
# env var MAESTRIA_DAEMON_PROFILE remains an alias for scripts.
# maestria start -i .maestria-dev --profile trusted-workspace
# Stop with Ctrl-C; start again picks up where it left off
```

### Local Studio

Studio is a daemon-first, authenticated loopback frontend. Start the matching
daemon, then launch the client with the instance explicitly selected:

```bash
maestria start -i .maestria-dev
maestria studio -i .maestria-dev --no-open
```

Without `--no-open`, the CLI asks the platform default browser to open the
printed `studio_url`. If the daemon is unavailable, the command exits with:
`daemon unavailable; start it with maestria start -i .maestria-dev`.

Studio reads agent profiles only from
`.maestria-dev/system/studio-agents.toml`; the CLI has no agent-config path
override and never reads a profile from the current working directory. The
file may configure an ACP-compatible external command:

```toml
default_agent = "omp"

[[agents]]
id = "omp"
label = "Oh My Pi"
command = "omp"
args = ["--no-tools", "--no-session", "acp"]
timeout_secs = 120
max_output_bytes = 65536
```

If the file is absent and `omp` is on `PATH`, Studio discovers the exact
in-memory profile `omp --no-tools --no-session acp`. If neither is available,
the notebook, search, citation, and draft UI remains usable while Ask reports
`agent_unconfigured`. Sillage is an ACP client: it does not ship, install,
update, authenticate, or implement an agent harness or model provider.

The URL carries an ephemeral bearer session fragment. Studio moves it into
origin-scoped session storage, removes it from the address bar, and sends it
as an Authorization bearer on API requests. Every Ask starts a fresh ACP v1
session with no filesystem, terminal, elicitation, boolean config-option, or
MCP capability. Only bounded text chunks and a terminal `end_turn` are
accepted. The agent must return exactly one validated JSON object; citations
are rebuilt from the daemon context and generated Markdown is never saved
automatically.

Questions run against each notebook's explicitly attached, currently indexed
and policy-allowed sources. Retrieval applies source selection before
candidate fusion, reranking, graph expansion, and evidence loading, and
returns a deterministic source-selection digest. Answers are rejected when
citations refer to evidence outside the returned context. Draft Markdown is
saved explicitly through typed notebook draft actions; the daemon remains
authoritative for source identity, evidence, provenance, and durable revisions.

The daemon notebook operations used by Studio are `notebook_list`,
`notebook_create`, `notebook_get`, `notebook_rename`, `notebook_delete`,
`notebook_source_catalog`, `notebook_source_attach`, `notebook_source_detach`,
`notebook_context`, `notebook_evidence`, `notebook_draft_list`,
`notebook_draft_get`, `notebook_draft_save`, and `notebook_draft_delete`.

### Local realm federation

Federation is an explicit, local, provider-owned read capability. It never
changes ordinary `search` or `open-evidence`: use the `realm` commands from a
consumer instance instead.

New instances are schema v2. Before using an existing schema-v1 instance,
migrate it once and retain the printed stable identity:

```bash
maestria realm migrate -i ~/provider
maestria realm migrate -i ~/consumer
maestria realm identity -i ~/provider
maestria realm identity -i ~/consumer
```

Start both local daemons, then have the provider issue and install a bounded
consumer binding:

```bash
maestria start -i ~/provider
maestria start -i ~/consumer

maestria realm grant create -i ~/provider \
  --consumer-instance ~/consumer \
  --access search-and-open-evidence \
  --max-sensitivity confidential \
  --max-results 1 \
  --max-evidence-bytes 128 \
  --expires-in-seconds 86400
```

The create command prints the grant digest but never the bearer credential. Use
the provider realm printed by `realm identity` for consumer reads, and revoke
with the digest when access is no longer required:

```bash
maestria realm search -i ~/consumer \
  --provider-realm <provider-realm-id> "provider-only phrase" --limit 1
maestria realm open-evidence -i ~/consumer \
  --provider-realm <provider-realm-id> --evidence-id 1
maestria realm grant list -i ~/provider
maestria realm grant revoke -i ~/provider <grant-token-digest>
```

Federation is Unix-socket-only. The provider keeps its daemon token; the
consumer receives a private, revocable capability that can search and,
only when granted, open a bounded provider evidence excerpt.

**Grant migration:** Schema v17 expires every older active realm read grant at
fixed Unix second 1. The original grant event log is unchanged; old credentials
cannot search or open evidence. On the provider, run `realm grant list`, revoke
the expired grant digest, and issue a new time-bounded grant for the same
consumer. Revoked grants remain revoked; access is never automatically renewed.

### Scoped local search API and separate search-only package

An approved and indexed provider can serve passage search from an unrelated
local process without a launcher, Studio, extension runtime or embedding
model. Start its daemon explicitly with
`maestria start -i ~/provider --profile read-only`. In another terminal,
issue a per-consumer capability. The credential is created once in a
mode-0600 file and is never printed:

```bash
consumer_realm="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
maestria realm grant create-external -i ~/provider \
  --consumer-realm "$consumer_realm" --credential-file ./search-client.key \
  --access search-only --max-sensitivity internal \
  --max-results 5 --max-evidence-bytes 256 --expires-in-seconds 3600
maestria search-api search \
  --socket-path ~/provider/system/daemon.sock \
  --consumer-realm "$consumer_realm" --credential-file ./search-client.key \
  --limit 5 "source-grounded phrase"
maestria search-api status \
  --socket-path ~/provider/system/daemon.sock \
  --consumer-realm "$consumer_realm" --credential-file ./search-client.key
```

`search-only` grants can receive bounded `preview.excerpt` text with a typed
`preview.location` and `preview.truncated` flag, but only a
`search-and-open-evidence` grant permits `search-api open-evidence`.
Grant revocation or expiry removes access; the provider instance token is
never given to the external client. The existing `maestria` commands above
are available from the full developer CLI. For a headless search-only build,
use the separate `maestria-search` binary and Debian package instead:

```bash
cargo build --release -p maestria-search
cargo install cargo-packager --locked --version 0.11.8
cargo packager --release --packages maestria-search --formats deb
# On a Debian-family host, install target/search-packages/maestria-search_*.deb
# with apt so system runtime dependencies are resolved.
maestria-search init --instance-dir ~/provider --read-root ~/Documents
maestria-search start --instance-dir ~/provider
# In a second terminal, generate a 64-character lowercase hexadecimal realm:
consumer_realm="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
maestria-search owner grant create-external --instance-dir ~/provider \
  --consumer-realm "$consumer_realm" --credential-file ./search-client.key \
  --access search-and-open-evidence --max-sensitivity internal \
  --max-results 5 --max-evidence-bytes 4096
maestria-search search --socket-path ~/provider/system/daemon.sock \
  --consumer-realm "$consumer_realm" --credential-file ./search-client.key \
  --limit 5 "source-grounded phrase"
maestria-search interactive-search --socket-path ~/provider/system/daemon.sock \
  --consumer-realm "$consumer_realm" --credential-file ./search-client.key \
  --limit 5 "source-grounded phrase"
# Pass an evidence_id from the result to maestria-search open-evidence
# with the same socket, realm and private credential file.
```

The provider's index has one owner; `start` uses the read-only profile without
a model client, and installation does not start a service or enable autostart.
Use `owner roots status|add|remove` to inspect or change approved read roots,
`owner grant list|revoke` to inspect or revoke grants, and
`indexing-status` to inspect live indexing freshness. A client without a
matching, live grant is denied. In a disposable Ubuntu 24.04 container the
apt-installed `/usr/bin/maestria-search` indexed Markdown, DOCX paragraphs and
a text-bearing PDF, reported an image-only PDF as OCR-needed, and served
bounded typed previews through general and v2 interactive search. Federated
evidence reopening returned an authorized current PDF source path and exact
page citation; previews exposed no PDF path. Grant denial/revocation, daemon
restart, and suppression of both a previously indexed passage and direct
reopening of its old evidence after the source changed passed locally.

On an uninstrumented, fully indexed 10,000-Markdown-file corpus, 197 of 200
warm exact-phrase interactive requests succeeded and three hit the existing
100 ms daemon deadline. Successful separate-process CLI requests had p50
68.05 ms, p95 78.42 ms, and p99 110.7 ms; these are **not** native
keystroke-to-passage timings. After approving an additional 700-file root,
indexing status reported 429 pending; the first six of 30 interactive
requests timed out and the other 24 succeeded (successful CLI p95 101.4 ms).
Indexing reached zero pending during that probe, so the successful subset
does not establish active-indexing responsiveness.

With direct-path lookup for file/DOCX source validation, a later isolated
200-call warm run on the retained, now fully indexed corpus succeeded 199
times; successful separate-process CLI p50/p95/p99 were 38.77/51.20/68.02 ms.
One call still timed out at 100 ms. A concurrent daemon-test run succeeded
196/200, with four timeouts. The workloads differ from the initial 10,000-file
probe, so these figures do not establish a causal speedup or deadline
acceptance.

Hosted CI, native whole-path latency, Wayland passage actions, relevance and
complete product acceptance remain open.

## Supported surfaces and capability status

### Daemon client

`maestria start -i <instance>` runs the local daemon. Its authenticated local
client boundary is newline-delimited JSON on
`<instance>/system/daemon.sock`; the token is stored in
`<instance>/system/daemon.token`.
The owner-only instance-token operations include `status`, `retrieval_status`, `search`,
`evidence`, `task`, `retire_retrieval_events`, `index_candidates`,
`index_selection_get`, `index_selection_save`, `index_run`,
`repository_index_candidates`, `repository_index_children`,
`repository_index_files`, `repository_index_progress_get`, `repository_index_run`,
`repository_index_selection_get`, `repository_index_selection_save`,
`repository_index_status`, `model_agent_propose`, `model_agent_status`,
`model_agent_resolve`, `realm_grant_create`, `realm_grant_list`, `realm_grant_revoke`,
`install_federation_binding`, and the notebook/draft operations
`notebook_list`, `notebook_create`, `notebook_get`, `notebook_rename`,
`notebook_delete`, `notebook_source_catalog`, `notebook_source_attach`,
`notebook_source_detach`, `notebook_context`, `notebook_evidence`,
`notebook_draft_list`, `notebook_draft_get`, `notebook_draft_save`, and
`notebook_draft_delete`. A federation credential can invoke only the separate
`federation_search` and `federation_evidence` provider operations.
Requests without the matching capability are rejected. Notebook mutations are
durable domain inputs and wait for their event/blob persistence barrier;
ordinary read-only operations cannot mutate domain state. Successful
federated reads append only their bounded access-audit event.
`model_agent_propose` is a bounded proposal workflow: it may search and request
harness execution, while `model_agent_status` reads its durable state and
`model_agent_resolve` records the operator decision. Governance, validation,
and approval still control every side effect. See
[`docs/DAEMON-API.md`](./docs/DAEMON-API.md) for typed errors, 64 KiB framing,
cancellation behavior, source-selection semantics, and request/response
envelopes.

### Repository and document retrieval

Repository indexing and bounded context queries are supported:

```bash
maestria index -i .maestria-dev repository ~/Projects/my-project
maestria search -i .maestria-dev code symbol "SearchPlan"
maestria search -i .maestria-dev code context "RetrievalEngine" --depth 2 --nodes 32
```

Cargo workspaces, Python distributions (`pyproject.toml`/`setup.cfg`/`setup.py`), and
web/TypeScript packages (`package.json`, workspaces discovered by walk) index into the
same repository projection: Rust, Python, and TS/JS symbols (modules, functions, JSX
components, classes, interfaces/types, imports) are searchable with the same
`search code` commands, and web lockfiles participate in the worktree identity.


PDF evidence preserves page/region provenance. Text/layout retrieval is the
stable route. Visual-provider retrieval is optional and remains shadowed unless
its frozen benchmark proves a quality and resource win; missing visual or OCR
providers degrade explicitly rather than fabricating text or coordinates.
Current-web queries require an enabled governed web adapter; without one they
use the bounded local fallback and expose the degradation in `search explain`.

Scanned PDFs can optionally use a local OCR adapter backed by ONNX Runtime.
The default remains provider-free: scanned pages stay `NeedsOcr` and no text
is fabricated. Visual retrieval uses the same optional local-provider
boundary: the portable profile is a CPU-only SigLIP ONNX runtime, with an
optional higher-quality visual-embedding profile. Neither model is required
for normal text/layout retrieval.

When enabled, both sidecars listen on loopback only, perform CPU inference,
and retain no inputs; Sillage never downloads or executes model code and
`maestria doctor` reports whether the configured rasterizer or visual
capability is available. Pinned sidecar profiles — revisions, artifact
hashes, endpoints, and manifest key blocks — are dated implementation
candidates documented in [`docs/RESEARCH.md`](./docs/RESEARCH.md); omit the
`ocr_*` and `visual_*` manifest keys to keep the capabilities disabled.

### Advanced existing workflows: tasks, validation, approvals, and memory

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
in the current build. Repository/code and visual-document features
are implemented but remain release-visible capability surfaces with explicit
freshness/provider degradation. Advanced dense, learned-sparse,
late-interaction, graph/temporal, and multimodal promotions are
benchmark-gated: unavailable or unproven routes abstain or use a bounded
local fallback, and research candidates are not silently promoted.

## Command reference

Every command accepts `-i, --instance-dir <PATH>` (default `.maestria-dev`).

### `init`

Create a local Sillage instance layout and manifest.

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
| `--max-file-bytes <N>` | Skip files larger than N bytes (0 disables) |
| `--skip-generated` | Skip generated asset dumps (single-extension dumps) |
| `--skip-minified` | Skip minified single-line bundles |
| `--yes` | Accept every directory prompt (non-interactive) |
| `--save-selection` | Write the approved selection to `system/index-selection.json` |

Directories are classified into Recommended / Maybe / Noise; Recommended
directories are whitelisted automatically, Noise subtrees are excluded, and
the rest are offered interactively with bounded drill-down (`Y`/`n`/`l`/`p`/
`a`/`q`). On a non-TTY run (or with `--yes`) every non-Noise directory is
whitelisted. Files outside the whitelist are never submitted.

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
maestria search [-i <dir>] roots status
maestria search [-i <dir>] roots add <directory>
maestria search [-i <dir>] roots remove <directory>
```

With the default instance, approve a root explicitly, inspect its index, and
revoke it when no longer needed:

```bash
maestria search roots add ~/Documents
maestria search roots status
maestria search roots remove ~/Documents
```

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory |
| `-l, --limit` | Max results (default 10) |


`search explain` executes a bounded search and prints its plan and trace.
`search trace` and `search compare` require durable, reproducible trace
payloads; missing or non-reproducible identifiers fail clearly.

Direct `search` runs daemon-first: it prints `served=daemon` when the
instance daemon served the query and `served=local` when it ran locally, so
benchmark and latency numbers are attributable (see
[`docs/OPERATIONS.md`](./docs/OPERATIONS.md) §7).

`search roots` is provider-owned administration: approval is explicit, and
removal immediately excludes the root from search and evidence opening.
`roots status` can list source paths for the owner. The daemon's watcher
reports files as indexed only after a durable parser receipt; queued work is
reported as pending. Edits, deletions, and reapproval of unchanged content
reconcile without broadening scope.

#### `search code`

Query the persisted repository code index built by `index repository`. All
`search code` commands search the same persisted index and share the
`-i`/`--instance-dir` and `-l`/`--limit` flags.

```
maestria search code symbol <pattern>
maestria search code path <pattern>
maestria search code regex <pattern>
maestria search code doc <pattern>
maestria search code markers <todo|fixme|hack|unsafe>
maestria search code changed [--since <commit>]
maestria search code references <pattern> [--direction inbound|outbound]
maestria search code context <pattern> [--depth <n>] [--nodes <n>] [--direction both|forward|reverse]
```

| Subcommand | Description |
|------------|-------------|
| `symbol` | Match repository symbols by name or qualified-name substring |
| `path` | Match repository symbols by source path substring |
| `regex` | Match repository symbols and paths with a regular expression |
| `doc` | Match repository symbols whose doc comment contains the pattern (from `///`, `//!`, and `#[doc]` attributes) |
| `markers` | Match repository symbols carrying a `todo`, `fixme`, `hack`, or `unsafe` source marker |
| `changed` | Match symbols in files changed since a commit (persisted delta without `--since`, live git diff plus dirty set with it) |
| `references` | Resolve cross-file symbol references from a seed (inbound callers/importers by default; `--direction outbound` for the symbols the seed uses) |
| `context` | Traverse bounded repository relations from a symbol seed |

| Flag | Description |
|------|-------------|
| `-i, --instance-dir` | Instance root directory |
| `-l, --limit` | Max results (default 20) |
| `--since` | Commit reference: full 40-hex SHA-1, short hex prefix, or `HEAD`-family ref (changed only) |
| `--depth` | Context traversal depth (default 2, context only) |
| `--nodes` | Max nodes in context response (default 64, context only) |
| `--direction` | References direction `inbound`/`outbound` (default `inbound`, references only; case-insensitive), or context traversal `both`/`forward`/`reverse` (default `both`, context only) |

The code index is built from Cargo metadata and Rust source files. It is
validated against the instance manifest read scope before indexing and
queried with live freshness checks. Repository/code features are implemented
in the current build but remain provider-dependent and freshness-degraded;
they are not first-product launcher functionality. See
[`docs/ROADMAP.md`](./docs/ROADMAP.md) for the canonical product milestones
and [`docs/RESEARCH.md`](./docs/RESEARCH.md) for dated retrieval evidence.


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

### `retire-retrieval-events`

Retire retrieval audit events strictly below a durable log sequence
(ADR-0009). Emits one governed append-only marker — nothing is deleted;
rows below the boundary stop being decoded at open, `status` reports
`retrieval_events_retired_through`, and retired trace lookups answer
explicitly. Requires a recorded `--reason`.

```
maestria retire-retrieval-events -i <dir> --before-sequence <n> --reason "<why>" [--yes]
```

### `start`

Start the Sillage daemon for the given instance.

```
maestria start [-i <dir>] [--profile read-only|trusted-workspace]
```

### `realm`

Manage explicit local federation. Schema-v1 instances must first run
`realm migrate`; normal local searches never cross a realm boundary.

```
maestria realm migrate [-i <instance>]
maestria realm identity [-i <instance>]
maestria realm grant create [-i <provider>] --consumer-instance <consumer> \
  --access search-only|search-and-open-evidence \
  --max-sensitivity public|internal|confidential|restricted \
  --max-results <1..100> --max-evidence-bytes <1..65536> \
  [--expires-in-seconds <1..31536000>]
maestria realm grant create-external [-i <provider>] \
  --consumer-realm <64-hex-id> --credential-file <path> \
  --access search-only|search-and-open-evidence \
  --max-sensitivity public|internal|confidential|restricted \
  --max-results <1..100> --max-evidence-bytes <1..65536> \
  [--expires-in-seconds <1..31536000>]
maestria realm grant list [-i <provider>]
maestria realm grant revoke [-i <provider>] <grant-token-digest>
maestria realm search [-i <consumer>] --provider-realm <realm-id> [-l <n>] <query>
maestria realm open-evidence [-i <consumer>] --provider-realm <realm-id> \
  --evidence-id <n>
```

`grant create` must run while both local daemons are available. It creates the
provider grant and installs the credential only in the consumer's private
binding. `grant list` and `grant revoke` are provider administration commands.
`realm search` and `realm open-evidence` use the consumer daemon and return
only provider-authorized, bounded data with provider realm provenance.

### `search-api`

One explicit request to a running provider daemon from a separate local
process. The caller supplies a private credential file and a stable consumer
realm ID; no instance token, Studio or launcher is required.

```
maestria search-api search --socket-path <provider-daemon.sock> \
  --consumer-realm <64-hex-id> --credential-file <path> [-l <n>] <query>
maestria search-api status --socket-path <provider-daemon.sock> \
  --consumer-realm <64-hex-id> --credential-file <path>
maestria search-api indexing-status --socket-path <provider-daemon.sock> \
  --consumer-realm <64-hex-id> --credential-file <path>
maestria search-api open-evidence --socket-path <provider-daemon.sock> \
  --consumer-realm <64-hex-id> --credential-file <path> --evidence-id <n>
```

`indexing-status` is a bounded aggregate with counts, exclusion reasons, and
format support; unlike owner `roots status`, it does not disclose paths.
Revoking the provider grant denies both search and evidence requests.

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
| `maestria-domain` | Kernel | Deterministic domain types, events, transitions, and effects |
| `maestria-governance` | Kernel | Scope, risk, approval, validation, freshness, trust, and security policy |
| `maestria-ports` | Kernel | Capability traits and deterministic in-memory contract adapters |
| `maestria-core` | Core | Local-first orchestration services and instance composition |
| `maestria-runtime` | Runtime | Effect execution, workers, queues, cancellation, retries, and journaling |
| `maestria-cli` | App | User-facing CLI binary |
| `maestria-daemon` | App | Restart-safe daemon with authenticated local API |
| `maestria-retrieval` | Ecosystem | Typed search planning, candidate generation, fusion, and reranking |
| `maestria-code-intel` | Ecosystem | Repository code intelligence index for workspace metadata and Rust symbols |
| `maestria-parsers` | Ecosystem | Source parsing and document structure extraction |
| `maestria-memory` | Ecosystem | Candidate deduplication, promotion workflow, and staleness handling |
| `maestria-validation` | Ecosystem | Validation runners, reports, and completion gating |
| `maestria-web-evidence` | Ecosystem | Governed web evidence fetching and current-web retrieval |
| `maestria-embedding-openai` | Ecosystem | OpenAI-compatible embedding provider adapter |
| `maestria-ocr-local` | Ecosystem | Local OCR provider adapter for scanned PDFs |
| `maestria-visual-local` | Ecosystem | Local visual retrieval provider adapter for page/region evidence |
| `maestria-harness` | Harness | Normalized external execution and capability reporting |
| `maestria-harness-cli` | Harness | CLI harness for local command execution |
| `maestria-storage-sqlite` | Storage | SQLite-based state persistence, event log, and repository traits |
| `maestria-search-tantivy` | Storage | Tantivy-based full-text lexical index |
| `maestria-vector-sqlite` | Storage | SQLite-based vector similarity index |
| `maestria-graph-sqlite` | Storage | SQLite-based graph projection index |
| `maestria-blob-fs` | Storage | Filesystem-backed immutable blob store |

## Invariants

- Domain and governance are side-effect free.
- All side effects are represented as typed intentions (effects).
- Policy and mechanism are separated by trait boundaries.
- Evidence is typed and source-grounded; raw strings are not evidence.
- Memory candidates point back to evidence. LLM output can propose; it cannot silently promote.

## Development

```bash
# Complete local gate: format, compile, lint, tests, docs,
# dependency, philosophy, and script checks.
cargo fmt --all -- --check
./scripts/strict-clippy.sh
cargo test --workspace --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
python3 scripts/philosophy-check.py
python3 scripts/codeowners-check.py
python3 scripts/doc-consistency-check.py
python3 -m unittest discover -s scripts -p 'test_*.py'
```


## Documentation map

- `docs/PHILOSOPHY.md` — repository doctrine
- `docs/SPECS.md` — invariant ledger
- [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) — system boundaries and ownership
- [`docs/SEARCH.md`](./docs/SEARCH.md) — typed retrieval contracts
- [`docs/MEMORY.md`](./docs/MEMORY.md) — source-backed memory lifecycle
- [`docs/SECURITY.md`](./docs/SECURITY.md) — scope, trust, taint, and secrets
- [`docs/OPERATIONS.md`](./docs/OPERATIONS.md) — runtime lifecycle and recovery
- [`docs/ROADMAP.md`](./docs/ROADMAP.md) — canonical product roadmap
- [`docs/BENCHMARKING.md`](./docs/BENCHMARKING.md) — measurement protocol for performance claims
- [`docs/RESEARCH.md`](./docs/RESEARCH.md) — dated non-normative evaluation candidates
- [`docs/architecture/`](./docs/architecture/) — architecture books
- `docs/first-pr-guide.md` — contributor onboarding

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md), [docs/first-pr-guide.md](./docs/first-pr-guide.md), and
[docs/PHILOSOPHY.md](./docs/PHILOSOPHY.md) for branch and review expectations.

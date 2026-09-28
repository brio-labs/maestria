#!/usr/bin/env bash
set -euo pipefail

PACKAGE_LAUNCHER=io-github-briolabs-maestria-launcher
PACKAGE_SEARCH=io-github-briolabs-maestria-search
PACKAGE_WORKER=io-github-briolabs-maestria-extension-worker
BINARY_LAUNCHER=/usr/bin/maestria-launcher
BINARY_SEARCH=/usr/bin/maestria-search
BINARY_WORKER=/usr/bin/maestria-extension-worker
OLD_SOURCE_REVISION=985a8368458d7267a4b150ae52702f90fbeedbcd
OLD_ARTIFACT_RUN=36455626998
OLD_SOURCE_UPSTREAM=0.0.0
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

fail() { echo "version-upgrade smoke failed: $*" >&2; exit 1; }

usage() {
  echo "usage: smoke-version-upgrade.sh OLD_LAUNCHER_DIR OLD_SEARCH_DIR OLD_WORKER_DIR NEW_LAUNCHER_DIR NEW_SEARCH_DIR NEW_WORKER_DIR" >&2
  exit 2
}

[[ $# -eq 6 || ( $# -eq 8 && ${7:-} == --inside ) ]] || usage
input_dirs=()
for directory in "${@:1:6}"; do
  [[ -d "$directory" ]] || fail "package directory does not exist: $directory"
  input_dirs+=("$(realpath -- "$directory")")
done

for tool in dpkg-deb dpkg-query dpkg apt-get sha256sum timeout realpath sudo git python3; do
  command -v "$tool" >/dev/null 2>&1 || fail "required tool is unavailable: $tool"
done

shopt -s nullglob
package_paths=()
for directory in "${input_dirs[@]}"; do
  packages=("$directory"/*.deb)
  [[ ${#packages[@]} -eq 1 ]] || fail "expected exactly one .deb package in $directory, found ${#packages[@]}"
  package_paths+=("${packages[0]}")
done

package_names=("$PACKAGE_LAUNCHER" "$PACKAGE_SEARCH" "$PACKAGE_WORKER")
binary_paths=("$BINARY_LAUNCHER" "$BINARY_SEARCH" "$BINARY_WORKER")
package_versions=()
package_sha256=()
package_upstreams=()
for index in 0 1 2; do
  old_index=$index
  new_index=$((index + 3))
  expected_package="${package_names[$index]}"
  old_package="${package_paths[$old_index]}"
  new_package="${package_paths[$new_index]}"
  for package in "$old_package" "$new_package"; do
    actual_package="$(dpkg-deb --field "$package" Package)" || fail "cannot read package name from $package"
    [[ "$actual_package" == "$expected_package" ]] || fail "expected $expected_package in $package, found $actual_package"
    architecture="$(dpkg-deb --field "$package" Architecture)" || fail "cannot read architecture from $package"
    [[ "$architecture" == amd64 ]] || fail "expected amd64 package $package, found $architecture"
    version="$(dpkg-deb --field "$package" Version)" || fail "cannot read version from $package"
    [[ -n "$version" ]] || fail "package version is empty in $package"
    dependencies="$(dpkg-deb --field "$package" Depends 2>/dev/null || true) $(dpkg-deb --field "$package" Pre-Depends 2>/dev/null || true) $(dpkg-deb --field "$package" Recommends 2>/dev/null || true) $(dpkg-deb --field "$package" Suggests 2>/dev/null || true)"
    for other_package in "${package_names[@]}"; do
      [[ "$other_package" == "$expected_package" ]] && continue
      [[ "$dependencies" != *"$other_package"* ]] || fail "$expected_package has a direct package coupling on $other_package in $package"
    done
    package_versions+=("$version")
    package_sha256+=("$(sha256sum -- "$package" | cut -d ' ' -f 1)")
    upstream="${version#*:}"
    upstream="${upstream%-*}"
    package_upstreams+=("$upstream")
  done
  old_version="${package_versions[$((2 * index))]}"
  new_version="${package_versions[$((2 * index + 1))]}"
  old_upstream="${package_upstreams[$((2 * index))]}"
  new_upstream="${package_upstreams[$((2 * index + 1))]}"
  [[ "$old_upstream" == "$OLD_SOURCE_UPSTREAM" ]] || fail "$expected_package old upstream version must be $OLD_SOURCE_UPSTREAM, found $old_upstream"
  [[ "$new_upstream" != "$old_upstream" ]] || fail "$expected_package old and new upstream versions are identical: $old_upstream"
  dpkg --compare-versions "$old_upstream" lt "$new_upstream" || fail "$expected_package upstream version is not newer: old=$old_upstream new=$new_upstream"
  [[ "${package_sha256[$((2 * index))]}" != "${package_sha256[$((2 * index + 1))]}" ]] || fail "$expected_package old and new Debian artifact SHA-256 are identical"
  if [[ ${7:-} == --inside ]]; then
    printf 'VERSION_UPGRADE_ARTIFACT package=%s old_version=%s old_sha256=%s new_version=%s new_sha256=%s\n' \
      "$expected_package" "$old_version" "${package_sha256[$((2 * index))]}" "$new_version" "${package_sha256[$((2 * index + 1))]}"
  fi
done

old_versions=("${package_versions[0]}" "${package_versions[2]}" "${package_versions[4]}")
new_versions=("${package_versions[1]}" "${package_versions[3]}" "${package_versions[5]}")
old_upstreams=("${package_upstreams[0]}" "${package_upstreams[2]}" "${package_upstreams[4]}")
new_upstreams=("${package_upstreams[1]}" "${package_upstreams[3]}" "${package_upstreams[5]}")
[[ "${old_versions[0]}" == "${old_versions[1]}" && "${old_versions[1]}" == "${old_versions[2]}" ]] || fail 'old launcher, search, and worker artifacts do not share one upstream package version'
[[ "${new_versions[0]}" == "${new_versions[1]}" && "${new_versions[1]}" == "${new_versions[2]}" ]] || fail 'new launcher, search, and worker artifacts do not share one upstream package version'
[[ "${old_upstreams[0]}" == "$OLD_SOURCE_UPSTREAM" && "${new_upstreams[0]}" == "${new_upstreams[1]}" && "${new_upstreams[1]}" == "${new_upstreams[2]}" ]] || fail 'artifact sets have inconsistent upstream product versions'

repo_root="$(realpath -- "$SCRIPT_DIR/..")"
checkout_revision="$(git -C "$repo_root" rev-parse --verify HEAD 2>/dev/null)" || fail 'cannot identify the checked-out committed source revision'
new_source_revision="${NEW_SOURCE_REVISION:-${GITHUB_SHA:-$checkout_revision}}"
[[ "$OLD_SOURCE_REVISION" =~ ^[0-9a-f]{40,64}$ && "$new_source_revision" =~ ^[0-9a-f]{40,64}$ ]] || fail 'source provenance must use full lowercase Git commit hashes'
[[ "$new_source_revision" == "$checkout_revision" ]] || fail "new artifact source revision $new_source_revision does not match checked-out source $checkout_revision"
[[ "$OLD_SOURCE_REVISION" != "$new_source_revision" ]] || fail 'old and new artifacts claim the same committed source revision'
if [[ -n "${GITHUB_SHA:-}" ]]; then
  [[ "$GITHUB_SHA" == "$checkout_revision" ]] || fail "GitHub artifact source $GITHUB_SHA differs from checked-out source $checkout_revision"
fi
git -C "$repo_root" cat-file -e "${OLD_SOURCE_REVISION}^{commit}" 2>/dev/null || fail 'old source commit is unavailable locally; fetch the release baseline before running this smoke'
product_source_diff="$(git -C "$repo_root" diff --name-only "$OLD_SOURCE_REVISION" "$new_source_revision" -- crates src launcher/src extension-sdk/src)" || fail 'cannot compare old and new committed product sources'
product_code_changed=false
while IFS= read -r changed_path; do
  case "$changed_path" in
    src/version.rs|*/tests/*|tests/*|*/test/*|*/__tests__/*|*/tests.rs|*/test_*.rs|*/*_test.rs) continue ;;
  esac
  case "$changed_path" in
    *.rs|*.ts|*.tsx|*.jsx|*.js|*.mjs|*.slint|*.css|*.html|*.wgsl) product_code_changed=true ;;
  esac
done <<<"$product_source_diff"
[[ "$product_code_changed" == true ]] || fail 'source revisions have no non-test product-code difference; refusing a metadata-only version upgrade'
if [[ ${7:-} == --inside ]]; then
  printf 'VERSION_UPGRADE_PRODUCT_CODE_DIFF=verified files=%s\n' "$product_source_diff"
fi
if [[ ${7:-} == --inside ]]; then
  printf 'VERSION_UPGRADE_PROVENANCE old_run=%s old_source_revision=%s new_source_revision=%s\n' \
    "$OLD_ARTIFACT_RUN" "$OLD_SOURCE_REVISION" "$new_source_revision"
fi
if [[ ${7:-} == --inside ]]; then
  [[ $# -eq 8 && -d "$8" ]] || usage
  root="$(realpath -- "$8")"
else
  for tool in xvfb-run dbus-run-session jwm xdotool xclip xprop gdbus; do
    command -v "$tool" >/dev/null 2>&1 || fail "required private desktop tool is unavailable: $tool"
  done
  umask 077
  root="$(mktemp -d "${TMPDIR:-/tmp}/maestria-version-upgrade.XXXXXX")"
  chmod 700 "$root"
  cleanup_outer() {
    # Installed extension bundles seal package directories read-only; after the
    # private launcher exits, restore owner write access only inside this root.
    local sealed="$root/data/io.github.briolabs.Maestria.Launcher/extensions/packages"
    if [[ -d "$sealed" && ! -L "$sealed" ]]; then chmod -R u+w -- "$sealed"; fi
    rm -rf -- "$root"
  }
  trap cleanup_outer EXIT
  mkdir -m 700 "$root/home" "$root/config" "$root/cache" "$root/data" "$root/state" "$root/runtime"
  export HOME="$root/home" XDG_CONFIG_HOME="$root/config" XDG_CACHE_HOME="$root/cache"
  export XDG_DATA_HOME="$root/data" XDG_STATE_HOME="$root/state" XDG_RUNTIME_DIR="$root/runtime"
  export XDG_DATA_DIRS=/usr/share XDG_CURRENT_DESKTOP=GNOME
  export PATH=/usr/bin:/bin LANG=C.UTF-8 LC_ALL=C.UTF-8 DEBIAN_FRONTEND=noninteractive
  export GDK_BACKEND=x11 WINIT_UNIX_BACKEND=x11
  unset WAYLAND_DISPLAY DISPLAY XAUTHORITY DBUS_SESSION_BUS_ADDRESS AT_SPI_BUS_ADDRESS
  timeout --kill-after=5s 600s xvfb-run -a -s '-screen 0 1280x1024x24' \
    dbus-run-session -- bash "$SCRIPT_DIR/smoke-version-upgrade.sh" "${input_dirs[@]}" --inside "$root"
  exit
fi

[[ -d "$root/home" && -d "$root/runtime" && "${XDG_RUNTIME_DIR:-}" == "$root/runtime" ]] || fail 'private XDG runtime is missing'
[[ "$(dpkg --print-architecture)" == amd64 ]] || fail 'this acceptance smoke requires an amd64 host'
. /etc/os-release
[[ "${ID:-}" == ubuntu && "${VERSION_ID:-}" == 24.04 ]] || fail 'installed upgrade acceptance must run on Ubuntu 24.04'
[[ "$HOME" == "$root/home" && "$XDG_CONFIG_HOME" == "$root/config" && "$XDG_CACHE_HOME" == "$root/cache" && "$XDG_DATA_HOME" == "$root/data" && "$XDG_STATE_HOME" == "$root/state" ]] || fail 'user state is not isolated in the private XDG root'
[[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]] || fail 'private D-Bus session is unavailable'
export PATH=/usr/bin:/bin AT_SPI_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"

for tool in bwrap jwm xdotool xclip xprop gdbus sudo setsid od tr sed stat; do
  command -v "$tool" >/dev/null 2>&1 || fail "required installed acceptance tool is unavailable: $tool"
done
registry=/usr/libexec/at-spi2-registryd
[[ -x "$registry" ]] || registry=/usr/lib/at-spi2-registryd
[[ -x "$registry" ]] || fail 'AT-SPI registry executable is unavailable'

launcher_pid= search_pid= wm_pid= registry_pid=
launcher_log="$root/launcher.log"
search_log="$root/search-daemon.log"
process_state() {
  local pid="$1" stat_line
  [[ -r "/proc/$pid/stat" ]] || return 0
  IFS= read -r stat_line < "/proc/$pid/stat" || return 0
  stat_line="${stat_line##*) }"
  printf '%s\n' "${stat_line%% *}"
}
process_is_live() {
  local state
  state="$(process_state "$1")"
  [[ -n "$state" && "$state" != Z && "$state" != X ]]
}

stop_search_best_effort() {
  [[ -n "$search_pid" ]] || return 0
  if process_is_live "$search_pid"; then
    kill -TERM "$search_pid" 2>/dev/null || true
    for attempt in {1..200}; do process_is_live "$search_pid" || break; sleep .05; done
    if process_is_live "$search_pid"; then kill -KILL "$search_pid" 2>/dev/null || true; fi
  fi
  wait "$search_pid" 2>/dev/null || true
  search_pid=
}
stop_launcher_best_effort() {
  [[ -n "$launcher_pid" ]] || return 0
  if process_is_live "$launcher_pid"; then
    timeout --kill-after=1s 5s "$BINARY_LAUNCHER" --quit >/dev/null 2>&1 || true
    for attempt in {1..100}; do process_is_live "$launcher_pid" || break; sleep .05; done
    if process_is_live "$launcher_pid"; then kill -TERM -- "-$launcher_pid" 2>/dev/null || kill -TERM "$launcher_pid" 2>/dev/null || true; fi
  fi
  wait "$launcher_pid" 2>/dev/null || true
  launcher_pid=
}
cleanup_inner() {
  local status=$?
  trap - EXIT INT TERM
  stop_launcher_best_effort
  stop_search_best_effort
  for pid in "$wm_pid" "$registry_pid"; do
    if [[ -n "$pid" ]]; then
      kill -TERM "$pid" 2>/dev/null || true
      for attempt in {1..100}; do process_is_live "$pid" || break; sleep .05; done
      if process_is_live "$pid"; then kill -KILL "$pid" 2>/dev/null || true; fi
      wait "$pid" 2>/dev/null || true
    fi
  done
  [[ ! -s "$launcher_log" ]] || cat "$launcher_log" >&2
  [[ ! -s "$search_log" ]] || cat "$search_log" >&2
  exit "$status"
}
trap cleanup_inner EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir -p "$root/config/io.github.briolabs.Maestria.Launcher" "$root/package/dist" "$root/search"
cat > "$root/config/io.github.briolabs.Maestria.Launcher/launcher.toml" <<'TOML'
schemaVersion = 1
shortcut = "Control+Space"
shortcutSetup = "deferred"
reduceMotion = true
TOML
chmod 600 "$root/config/io.github.briolabs.Maestria.Launcher/launcher.toml"
config_file="$root/config/io.github.briolabs.Maestria.Launcher/launcher.toml"

cat > "$root/package/manifest.json" <<'JSON'
{"apiVersion":1,"id":"dev.sillage.version-upgrade","name":"Version Upgrade Broker","version":"1.0.0","entrypoints":[{"id":"main","file":"dist/main.js"}],"commands":[{"id":"state-check","title":"Version upgrade state","entrypointId":"main"}],"permissions":[{"type":"copy","formats":["text"]},{"type":"storage","scope":"extension"}]}
JSON
cat > "$root/package/dist/main.js" <<'JS'
const stateKey = "upgrade-proof";
const stateValue = "private-extension-state-survived-real-package-upgrade";
const clipboardValue = "private-xvfb-version-upgrade-copy-proof";

export default {
  commands: {
    "state-check": async ({ invocation, requestCapability }) => {
      if (invocation.kind === "action" && invocation.actionId === "initialize-state") {
        const saved = await requestCapability({ capability: "storage", operation: "set", key: stateKey, value: stateValue });
        if (!saved.ok) return { kind: "error", title: "State initialization denied", message: saved.error.message };
        const copied = await requestCapability({ capability: "copy", text: clipboardValue });
        if (!copied.ok) return { kind: "error", title: "Copy grant denied", message: copied.error.message };
        return { kind: "detail", title: "UPGRADE_PRIVATE_STATE_INITIALIZED_AND_COPY_ALLOWED", blocks: [{ kind: "text", text: stateValue }] };
      }
      if (invocation.kind === "action" && invocation.actionId === "verify-state") {
        const stored = await requestCapability({ capability: "storage", operation: "get", key: stateKey });
        const denied = await requestCapability({ capability: "fileSearch", query: "ungranted version upgrade root", limit: 1 });
        if (!stored.ok || stored.value !== stateValue) {
          return { kind: "error", title: "Persisted state missing", message: "The private extension value did not survive package replacement." };
        }
        if (denied.ok || denied.error.code !== "permission_denied") {
          return { kind: "error", title: "Unrequested file search was not denied", message: "Extension requested ungranted file access." };
        }
        const copied = await requestCapability({ capability: "copy", text: clipboardValue });
        if (!copied.ok) return { kind: "error", title: "Copy grant was not retained", message: copied.error.message };
        return { kind: "detail", title: "UPGRADE_PRIVATE_STATE_VERIFIED_AND_COPY_ALLOWED", blocks: [{ kind: "text", text: stateValue }] };
      }
      const stored = await requestCapability({ capability: "storage", operation: "get", key: stateKey });
      const denied = await requestCapability({ capability: "fileSearch", query: "ungranted version upgrade root", limit: 1 });
      const retained = stored.ok && stored.value === stateValue;
      const searchDenied = !denied.ok && denied.error.code === "permission_denied";
      return {
        kind: "list",
        title: retained && searchDenied ? "UPGRADE_STATE_RETAINED_AND_UNGRANTED_FILE_SEARCH_DENIED" : "UPGRADE_PRIVATE_STATE_NOT_INITIALIZED",
        items: [{
          id: "private-state",
          title: retained ? "Verify retained private upgrade state" : "Initialize private upgrade state",
          actions: [{ id: retained ? "verify-state" : "initialize-state", label: retained ? "Verify retained private upgrade state" : "Initialize private upgrade state", role: "primary" }]
        }]
      };
    }
  }
};
JS

registry_log="$root/at-spi-registry.log"
"$registry" >"$registry_log" 2>&1 & registry_pid=$!
registry_ready=false
for attempt in {1..100}; do
  owner="$(timeout --kill-after=1s 2s gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner org.a11y.atspi.Registry 2>/dev/null || true)"
  if [[ "$owner" == *true* ]]; then registry_ready=true; break; fi
  process_is_live "$registry_pid" || { cat "$registry_log" >&2; fail 'private AT-SPI registry exited before readiness'; }
  sleep .05
done
[[ "$registry_ready" == true ]] || { cat "$registry_log" >&2; fail 'private AT-SPI registry did not become ready'; }
gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
  --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled '<true>' >/dev/null
printf '%s\n' '<JWM><FocusModel>click</FocusModel></JWM>' >"$root/jwm.xml"
jwm -f "$root/jwm.xml" >"$root/jwm.log" 2>&1 & wm_pid=$!
wm_ready=false
for attempt in {1..100}; do
  wm_check="$(timeout --kill-after=1s 2s xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null || true)"
  if [[ "$wm_check" == *'window id # 0x'* ]]; then wm_ready=true; break; fi
  process_is_live "$wm_pid" || { cat "$root/jwm.log" >&2; fail 'private X11 window manager exited before readiness'; }
  sleep .05
done
[[ "$wm_ready" == true ]] || { cat "$root/jwm.log" >&2; fail 'private X11 window manager did not become ready'; }

sandbox_args=(
  --unshare-user --unshare-pid --unshare-net --unshare-ipc --unshare-uts
  --disable-userns --cap-drop ALL --die-with-parent --new-session --clearenv
  --setenv HOME /home/extension --setenv LANG C.UTF-8 --setenv PATH /usr/bin
  --ro-bind /usr /usr --ro-bind-try /lib /lib --ro-bind-try /lib64 /lib64
  --proc /proc --dev /dev --dir /home --size 16777216 --tmpfs /tmp
  --dir /extension --chdir /extension
)
if ! timeout --kill-after=2s 10s bwrap "${sandbox_args[@]}" -- /usr/bin/true >"$root/bwrap-preflight.log" 2>&1; then
  cat "$root/bwrap-preflight.log" >&2
  fail 'required Bubblewrap user/pid/network namespace preflight failed; refusing an unsandboxed extension run'
fi
printf 'VERSION_UPGRADE_BWRAP_PREFLIGHT=passed\n'

verify_installed_set() {
  local set="$1" index expected_version expected_architecture installed owner binary_hash
  local -a versions
  if [[ "$set" == old ]]; then versions=("${old_versions[@]}"); else versions=("${new_versions[@]}"); fi
  for index in 0 1 2; do
    expected_version="${versions[$index]}"
    expected_architecture=amd64
    installed="$(dpkg-query --show --showformat='${Status}|${Version}|${Architecture}' "${package_names[$index]}" 2>/dev/null || true)"
    [[ "$installed" == "install ok installed|$expected_version|$expected_architecture" ]] || fail "$set ${package_names[$index]} is not installed from the exact expected artifact: $installed"
    binary="${binary_paths[$index]}"
    [[ -x "$binary" && "$(realpath -- "$binary")" == "$binary" ]] || fail "$set installed executable is missing or not the expected system path: $binary"
    owner="$(dpkg-query --search "$binary" 2>/dev/null || true)"
    [[ "$owner" == "${package_names[$index]}: $binary" ]] || fail "$set executable is not owned by its exact Debian package: $binary ($owner)"
    binary_hash="$(sha256sum -- "$binary" | cut -d ' ' -f 1)"
    printf 'VERSION_UPGRADE_INSTALLED set=%s package=%s version=%s binary=%s binary_sha256=%s owner=%s\n' \
      "$set" "${package_names[$index]}" "$expected_version" "$binary" "$binary_hash" "$owner"
    if [[ "$set" == new ]]; then
      old_hash="${old_binary_sha256[$index]}"
      [[ "$binary_hash" != "$old_hash" ]] || fail "${package_names[$index]} old and new installed executable bytes are identical"
      if [[ -n "${new_binary_sha256[$index]:-}" ]]; then
        [[ "$binary_hash" == "${new_binary_sha256[$index]}" ]] || fail "${package_names[$index]} reinstall changed the new package executable bytes"
      else
        new_binary_sha256[$index]="$binary_hash"
      fi
    else
      old_binary_sha256[$index]="$binary_hash"
    fi
  done
}

apt_log="$root/apt.log"
run_apt() {
  local label="$1"
  shift
  if ! timeout --kill-after=5s 180s sudo -n apt-get "$@" >"$apt_log" 2>&1; then
    cat "$apt_log" >&2
    fail "apt-get $label failed"
  fi
  cat "$apt_log"
}

private_launcher_window() {
  local attempt window=
  for attempt in {1..150}; do
    window="$(timeout --kill-after=1s 3s xdotool search --onlyvisible --name '^Sillage Launcher$' 2>/dev/null | { read -r id; printf '%s' "$id"; } || true)"
    [[ -n "$window" ]] && { printf '%s\n' "$window"; return 0; }
    process_is_live "$launcher_pid" || { cat "$launcher_log" >&2; fail 'installed launcher exited before showing its private window'; }
    sleep .1
  done
  cat "$launcher_log" >&2
  fail 'installed launcher did not show a private Xvfb window within 15 seconds'
}
start_launcher() {
  : >"$launcher_log"
  setsid "$BINARY_LAUNCHER" --activate >"$launcher_log" 2>&1 & launcher_pid=$!
  launcher_window="$(private_launcher_window)"
  timeout --kill-after=1s 5s xdotool windowactivate --sync "$launcher_window"
  printf 'VERSION_UPGRADE_LAUNCHER_STARTED path=%s pid=%s window=%s\n' "$BINARY_LAUNCHER" "$launcher_pid" "$launcher_window"
}
stop_launcher_gracefully() {
  [[ -n "$launcher_pid" ]] || return 0
  local status=0
  timeout --kill-after=1s 10s "$BINARY_LAUNCHER" --quit >/dev/null || { cat "$launcher_log" >&2; fail 'installed resident launcher --quit request failed'; }
  for attempt in {1..200}; do process_is_live "$launcher_pid" || break; sleep .05; done
  if process_is_live "$launcher_pid"; then fail 'resident launcher did not stop gracefully after --quit'; fi
  wait "$launcher_pid" || status=$?
  [[ "$status" -eq 0 ]] || fail "resident launcher exited with status $status after --quit"
  launcher_pid=
  printf 'VERSION_UPGRADE_LAUNCHER_STOPPED_GRACEFULLY exit_status=%s\n' "$status"
}

search_root="$root/search"
instance="$search_root/instance"
approved_root="$search_root/approved-root"
ungranted_root="$search_root/ungranted-root"
credential_file="$search_root/consumer.credential"
socket_path="$instance/system/daemon.sock"
primary_phrase='The silver kestrel carries an upgrade-proof compass through quiet rain'
ungranted_phrase='A violet meteor hides the ungranted upgrade-only archive'
fixture="$approved_root/upgrade-passage.md"
ungranted_fixture="$ungranted_root/private-note.md"
mkdir -m 700 "$approved_root" "$ungranted_root"
printf '%s\n' '# Durable version upgrade passage' "$primary_phrase." >"$fixture"
printf '%s\n' '# Unapproved private root' "$ungranted_phrase." >"$ungranted_fixture"
fixture="$(realpath -- "$fixture")"
approved_root="$(realpath -- "$approved_root")"
ungranted_root="$(realpath -- "$ungranted_root")"

search_cli() { timeout --kill-after=1s 12s "$BINARY_SEARCH" "$@"; }

setup_search_instance() {
  mkdir -m 700 "$instance"
  if ! search_cli init --instance-dir "$instance" --read-root "$approved_root" >"$search_root/init.out" 2>"$search_root/init.err"; then
    cat "$search_root/init.err" >&2
    fail 'installed search binary could not initialize the one private search instance'
  fi
  provider_realm="$(sed -n 's/^realm_id=//p' "$instance/manifest.txt")"
  [[ "$provider_realm" =~ ^[0-9a-f]{64}$ ]] || fail 'private search instance has no valid provider realm'
  consumer_realm="$(printf '%064x' 1)"
  [[ "$consumer_realm" != "$provider_realm" ]] || consumer_realm="$(printf '%064x' 2)"
  ungranted_realm="$(printf '%064x' 3)"
  [[ "$ungranted_realm" != "$provider_realm" && "$ungranted_realm" != "$consumer_realm" ]] || ungranted_realm="$(printf '%064x' 4)"
}

create_search_grant() {
  if ! grant_output="$(search_cli owner grant create-external \
    --instance-dir "$instance" --consumer-realm "$consumer_realm" \
    --credential-file "$credential_file" --access search-and-open-evidence \
    --max-sensitivity internal --read-root "$approved_root" --max-results 1 \
    --max-evidence-bytes 512 --expires-in-seconds 1800 2>"$search_root/grant-create.err")"; then
    cat "$search_root/grant-create.err" >&2
    fail 'installed search binary could not create one root-scoped consumer grant'
  fi
  [[ "$grant_output" == *"consumer_realm=$consumer_realm"* && "$grant_output" == *'state=active'* ]] || fail 'external grant output did not confirm the one consumer grant'
  [[ "$grant_output" == *"allowed_roots=[\"$approved_root\"]"* ]] || fail 'consumer grant did not freeze exactly the approved private root'
  grant_digest="$(sed -n 's/^grant_token_digest=//p' <<<"$grant_output")"
  [[ "$grant_digest" =~ ^[0-9a-f]{64}$ && -s "$credential_file" ]] || fail 'private search grant lacks its digest or credential'
  [[ "$(stat -c '%a' "$credential_file")" == 600 ]] || fail 'search credential is not mode 0600'
  consumer_args=(--socket-path "$socket_path" --consumer-realm "$consumer_realm" --credential-file "$credential_file")
  ungranted_args=(--socket-path "$socket_path" --consumer-realm "$ungranted_realm" --credential-file "$credential_file")
  printf 'VERSION_UPGRADE_SEARCH_INSTANCE path=%s provider_realm=%s consumer_realm=%s grant_digest=%s approved_root=%s ungranted_root=%s\n' \
    "$instance" "$provider_realm" "$consumer_realm" "$grant_digest" "$approved_root" "$ungranted_root"
}

wait_for_search_index() {
  local attempt indexed=false
  for attempt in {1..1200}; do
    if search_cli indexing-status "${consumer_args[@]}" >"$search_root/indexing-status.json" 2>"$search_root/indexing-status.err"; then
      if python3 - "$search_root/indexing-status.json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
data = response.get("data")
if response.get("type") == "indexing_status" and isinstance(data, dict) and data.get("approved_root_count") == 1 and data.get("indexed_file_count") == 1 and data.get("pending_file_count") == 0 and data.get("scanning") is False and data.get("last_scan_error") is False:
    raise SystemExit(0)
raise SystemExit(1)
PY
      then indexed=true; break; fi
    elif ! process_is_live "$search_pid"; then cat "$search_log" >&2; fail 'private search daemon stopped while indexing'; fi
    sleep .1
  done
  [[ "$indexed" == true ]] || { cat "$search_root/indexing-status.json" >&2 || true; cat "$search_log" >&2; fail 'one approved Markdown file did not finish indexing within 120 seconds'; }
}

start_search_daemon() {
  local attempt ready=false
  : >"$search_log"
  "$BINARY_SEARCH" start --instance-dir "$instance" >"$search_log" 2>&1 & search_pid=$!
  for attempt in {1..600}; do
    if search_cli owner roots status --instance-dir "$instance" >"$search_root/owner-roots.json" 2>"$search_root/owner-roots.err"; then
      if python3 - "$search_root/owner-roots.json" "$approved_root" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as stream:
    status = json.load(stream)
roots = status.get("roots")
if status.get("approved_root_count") != 1 or not isinstance(roots, list) or len(roots) != 1 or roots[0].get("path") != sys.argv[2]:
    raise SystemExit(1)
PY
      then ready=true; break; fi
    fi
    process_is_live "$search_pid" || { cat "$search_log" >&2; fail 'private search daemon exited during startup'; }
    sleep .05
  done
  [[ "$ready" == true && -S "$socket_path" ]] || { cat "$search_log" >&2; fail 'private search daemon did not bind its socket with exactly one approved root'; }
  printf 'VERSION_UPGRADE_SEARCH_DAEMON_STARTED binary=%s pid=%s socket=%s\n' "$BINARY_SEARCH" "$search_pid" "$socket_path"
  if [[ -s "$credential_file" ]]; then wait_for_search_index; fi
}

stop_search_gracefully() {
  [[ -n "$search_pid" ]] || return 0
  local status=0
  kill -INT "$search_pid" 2>/dev/null || fail 'cannot request graceful search daemon shutdown'
  for attempt in {1..200}; do process_is_live "$search_pid" || break; sleep .05; done
  if process_is_live "$search_pid"; then fail 'search daemon failed to stop gracefully (SIGKILL is not accepted)'; fi
  wait "$search_pid" || status=$?
  [[ "$status" -eq 0 ]] || fail "search daemon exited with status $status after SIGINT"
  search_pid=
  [[ ! -e "$socket_path" ]] || fail 'gracefully stopped search daemon left its private socket behind'
  printf 'VERSION_UPGRADE_SEARCH_DAEMON_STOPPED_GRACEFULLY exit_status=%s\n' "$status"
}

verify_search_behavior() {
  local phase="$1" expected_id="${2:-}" evidence_id
  local response="$search_root/$phase-search.json"
  if ! search_cli search "${consumer_args[@]}" --limit 1 "$primary_phrase" >"$response" 2>"$search_root/$phase-search.err"; then
    cat "$search_root/$phase-search.err" >&2
    fail "$phase approved search query failed"
  fi
  evidence_id="$(python3 - "$response" "$primary_phrase" "$fixture" "$expected_id" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
if response.get("type") != "search" or not isinstance(response.get("data"), dict) or response["data"].get("query") != sys.argv[2]:
    raise SystemExit("search response did not preserve its exact authorized query")
evidence = response["data"].get("evidence")
if not isinstance(evidence, list) or len(evidence) != 1:
    raise SystemExit("authorized passage did not produce exactly one bounded result")
item = evidence[0]
preview = item.get("preview") or {}
location = preview.get("location") or {}
if sys.argv[2] not in preview.get("excerpt", "") or location.get("path") != sys.argv[3] or location.get("type") != "file" or location.get("start_line", 0) < 1 or location.get("end_line", 0) < location.get("start_line", 0):
    raise SystemExit("search evidence did not cite the exact approved passage path")
if len(preview["excerpt"].encode("utf-8")) > 512:
    raise SystemExit("search evidence exceeded the consumer grant's byte limit")
evidence_id = item.get("evidence_id")
if not isinstance(evidence_id, int) or evidence_id <= 0:
    raise SystemExit("search result did not contain a valid evidence ID")
if sys.argv[4] and str(evidence_id) != sys.argv[4]:
    raise SystemExit(f"persisted evidence ID changed: expected {sys.argv[4]}, found {evidence_id}")
print(evidence_id)
PY
)" || fail "$phase approved search result was not the exact durable indexed passage"
  if [[ -z "$expected_id" ]]; then primary_evidence_id="$evidence_id"; fi

  if ! search_cli interactive-search "${consumer_args[@]}" --limit 1 "$primary_phrase" >"$search_root/$phase-interactive.json" 2>"$search_root/$phase-interactive.err"; then
    cat "$search_root/$phase-interactive.err" >&2
    fail "$phase installed interactive-search request failed"
  fi
  python3 - "$search_root/$phase-interactive.json" "$primary_phrase" "$fixture" "$primary_evidence_id" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
if response.get("type") != "search" or response.get("data", {}).get("query") != sys.argv[2]:
    raise SystemExit("interactive search did not preserve the exact query")
evidence = response["data"].get("evidence", [])
if len(evidence) != 1 or evidence[0].get("evidence_id") != int(sys.argv[4]):
    raise SystemExit("interactive search did not return the same bounded indexed passage")
preview = evidence[0].get("preview") or {}
if sys.argv[2] not in preview.get("excerpt", "") or (preview.get("location") or {}).get("path") != sys.argv[3]:
    raise SystemExit("interactive search returned a different/unapproved source")
PY
  if ! search_cli open-evidence "${consumer_args[@]}" --evidence-id "$evidence_id" >"$search_root/$phase-open.json" 2>"$search_root/$phase-open.err"; then
    cat "$search_root/$phase-open.err" >&2
    fail "$phase authorized evidence open failed"
  fi
  python3 - "$search_root/$phase-open.json" "$primary_phrase" "$fixture" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
if response.get("type") != "evidence" or not isinstance(response.get("data"), dict):
    raise SystemExit("open-evidence returned an unexpected type")
evidence = response["data"]
source = evidence.get("source") or {}
if sys.argv[2] not in evidence.get("excerpt", "") or source.get("type") != "file" or source.get("path") != sys.argv[3] or len(evidence["excerpt"].encode("utf-8")) > 512:
    raise SystemExit("open-evidence did not preserve the authorized bounded passage")
PY

  if ! search_cli search "${consumer_args[@]}" --limit 1 "$ungranted_phrase" >"$search_root/$phase-ungranted-root.json" 2>"$search_root/$phase-ungranted-root.err"; then
    cat "$search_root/$phase-ungranted-root.err" >&2
    fail "$phase authorized consumer could not query for a passage from the ungranted root"
  fi
  python3 - "$search_root/$phase-ungranted-root.json" "$ungranted_root" "$fixture" "$ungranted_phrase" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as stream:
    response = json.load(stream)
if response.get("type") != "search" or not isinstance(response.get("data"), dict):
    raise SystemExit("ungranted-root query returned an unexpected response")
data = response["data"]
evidence = data.get("evidence")
paths = data.get("path_results", [])
if not isinstance(evidence, list) or not isinstance(paths, list):
    raise SystemExit("ungranted-root search omitted its evidence/path boundary")
for item in evidence:
    preview = item.get("preview") or {}
    source = preview.get("location") or {}
    if source.get("path") != sys.argv[3] or sys.argv[4] in preview.get("excerpt", ""):
        raise SystemExit(f"consumer received an ungranted passage: {item!r}")
root_prefix = sys.argv[2] + "/"
for item in paths:
    path = item.get("path", "")
    if path == sys.argv[2] or path.startswith(root_prefix):
        raise SystemExit(f"consumer received a path result from the ungranted root: {item!r}")
PY
  if search_cli search "${ungranted_args[@]}" --limit 1 "$primary_phrase" >"$search_root/$phase-ungranted-consumer.stdout" 2>"$search_root/$phase-ungranted-consumer.stderr"; then
    fail "$phase unauthorized consumer realm unexpectedly searched the approved passage"
  fi
  [[ ! -s "$search_root/$phase-ungranted-consumer.stdout" && "$(<"$search_root/$phase-ungranted-consumer.stderr")" == *Unauthorized* ]] || {
    cat "$search_root/$phase-ungranted-consumer.stderr" >&2
    fail "$phase ungranted consumer realm did not receive typed Unauthorized"
  }
  printf 'VERSION_UPGRADE_SEARCH_VERIFIED phase=%s evidence_id=%s approved_path=%s ungranted_root=%s ungranted_consumer=Unauthorized\n' \
    "$phase" "$evidence_id" "$fixture" "$ungranted_root"
}

hash_tree() {
  python3 - "$1" <<'PY'
import hashlib
import stat
import sys
from pathlib import Path
root = Path(sys.argv[1])
if not root.is_dir() or root.is_symlink():
    raise SystemExit(f"private persistent state directory is missing or not a real directory: {root}")
digest = hashlib.sha256()
for path in sorted(root.rglob("*"), key=lambda item: item.relative_to(root).as_posix()):
    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not (stat.S_ISREG(metadata.st_mode) or stat.S_ISDIR(metadata.st_mode)):
        raise SystemExit(f"unexpected link or special file in persistent state: {path}")
    relative = path.relative_to(root).as_posix().encode()
    digest.update(len(relative).to_bytes(8, "big")); digest.update(relative)
    digest.update(stat.S_IMODE(metadata.st_mode).to_bytes(4, "big"))
    if stat.S_ISREG(metadata.st_mode):
        content = path.read_bytes()
        digest.update(len(content).to_bytes(8, "big")); digest.update(content)
print(digest.hexdigest())
PY
}
extension_store="$root/data/io.github.briolabs.Maestria.Launcher/extensions"
state_key_hex="$(printf '%s' upgrade-proof | od -An -tx1 | tr -d ' \n')"
# ActiveBundle supplies its per-extension data directory to the broker, which
# creates a second extension-scoped subdirectory. Read the old package's actual
# durable path so an upgrade must retain its value, not silently migrate it.
extension_data_file="$extension_store/data/dev.sillage.version-upgrade/dev.sillage.version-upgrade/$state_key_hex"

snapshot_private_state() {
  local label="$1"
  [[ -s "$config_file" ]] || fail "$label launcher settings file is missing"
  [[ "$(stat -c '%a' "$config_file")" == 600 ]] || fail "$label launcher settings file is not private mode 0600"
  [[ -s "$extension_data_file" && "$(<"$extension_data_file")" == private-extension-state-survived-real-package-upgrade ]] || fail "$label extension private storage value is missing or changed"
  [[ "$(stat -c '%a' "$extension_data_file")" == 600 ]] || fail "$label extension state file is not private mode 0600"
  config_sha="$(sha256sum -- "$config_file" | cut -d ' ' -f 1)"
  extension_sha="$(hash_tree "$extension_store")"
  search_sha="$(hash_tree "$instance")"
  credential_sha="$(sha256sum -- "$credential_file" | cut -d ' ' -f 1)"
  printf 'VERSION_UPGRADE_PRIVATE_STATE phase=%s launcher_settings_sha256=%s extension_permissions_and_data_sha256=%s search_instance_and_grants_sha256=%s consumer_credential_sha256=%s\n' \
    "$label" "$config_sha" "$extension_sha" "$search_sha" "$credential_sha"
}

assert_private_state_matches() {
  local label="$1" expected_config="$2" expected_extension="$3" expected_search="$4" expected_credential="$5"
  [[ "$(sha256sum -- "$config_file" | cut -d ' ' -f 1)" == "$expected_config" ]] || fail "$label modified private launcher settings"
  [[ "$(hash_tree "$extension_store")" == "$expected_extension" ]] || fail "$label modified extension permissions/package/data"
  [[ "$(hash_tree "$instance")" == "$expected_search" ]] || fail "$label modified the private search instance/consumer grant"
  [[ "$(sha256sum -- "$credential_file" | cut -d ' ' -f 1)" == "$expected_credential" ]] || fail "$label modified the external consumer credential"
  [[ "$(<"$extension_data_file")" == private-extension-state-survived-real-package-upgrade ]] || fail "$label changed the persistent extension value"
  printf 'VERSION_UPGRADE_PRIVATE_STATE_UNCHANGED phase=%s launcher_settings_sha256=%s extension_store_sha256=%s search_instance_sha256=%s consumer_credential_sha256=%s\n' \
    "$label" "$expected_config" "$expected_extension" "$expected_search" "$expected_credential"
}

for package in "${package_names[@]}"; do
  status="$(dpkg-query --show --showformat='${Status}' "$package" 2>/dev/null || true)"
  [[ "$status" != 'install ok installed' ]] || fail "runner already has $package installed; refusing a pre-existing package state"
done
run_apt 'install the old artifacts from the committed 0.0.0 source' install --yes --no-install-recommends "${package_paths[0]}" "${package_paths[1]}" "${package_paths[2]}"
old_binary_sha256=() new_binary_sha256=()
verify_installed_set old

setup_search_instance
start_search_daemon
create_search_grant
wait_for_search_index
verify_search_behavior old ""

start_launcher
window="$launcher_window"
timeout --kill-after=1s 45s python3 "$SCRIPT_DIR/check-version-upgrade.py" prepare "$root" "$window"
[[ -s "$extension_data_file" ]] || fail 'installed extension worker did not create its private XDG storage entry'
[[ "$(<"$extension_data_file")" == private-extension-state-survived-real-package-upgrade ]] || fail 'installed extension worker stored an unexpected value'
[[ "$(stat -c '%a' "$extension_data_file")" == 600 ]] || fail 'installed extension worker data is not mode 0600'
stop_launcher_gracefully
stop_search_gracefully
snapshot_private_state before-upgrade
saved_config_sha="$config_sha" saved_extension_sha="$extension_sha" saved_search_sha="$search_sha" saved_credential_sha="$credential_sha"


run_apt 'upgrade to new source-built Debian artifacts' install --yes --no-install-recommends "${package_paths[3]}" "${package_paths[4]}" "${package_paths[5]}"
verify_installed_set new
assert_private_state_matches after-upgrade-before-restart "$saved_config_sha" "$saved_extension_sha" "$saved_search_sha" "$saved_credential_sha"

start_search_daemon
verify_search_behavior after-upgrade "$primary_evidence_id"
start_launcher
window="$launcher_window"
timeout --kill-after=1s 45s python3 "$SCRIPT_DIR/check-version-upgrade.py" verify "$root" "$window"
stop_launcher_gracefully
stop_search_gracefully

# Package removal is deliberately separate from upgrade; retain, never purge, the private XDG state.
snapshot_private_state before-remove-reinstall
saved_config_sha="$config_sha" saved_extension_sha="$extension_sha" saved_search_sha="$search_sha" saved_credential_sha="$credential_sha"
run_apt 'remove all three independent product packages while retaining private user state' remove --yes --no-install-recommends "$PACKAGE_LAUNCHER" "$PACKAGE_SEARCH" "$PACKAGE_WORKER"
for binary in "${binary_paths[@]}"; do [[ ! -e "$binary" ]] || fail "package removal left installed executable $binary"; done
for package in "${package_names[@]}"; do
  status="$(dpkg-query --show --showformat='${Status}' "$package" 2>/dev/null || true)"
  [[ "$status" != 'install ok installed' ]] || fail "apt remove left $package installed"
done
assert_private_state_matches after-remove "$saved_config_sha" "$saved_extension_sha" "$saved_search_sha" "$saved_credential_sha"
run_apt 'reinstall the exact new product artifacts' install --yes --no-install-recommends "${package_paths[3]}" "${package_paths[4]}" "${package_paths[5]}"
verify_installed_set new
assert_private_state_matches after-reinstall "$saved_config_sha" "$saved_extension_sha" "$saved_search_sha" "$saved_credential_sha"

start_search_daemon
verify_search_behavior after-reinstall "$primary_evidence_id"
start_launcher
window="$launcher_window"
timeout --kill-after=1s 45s python3 "$SCRIPT_DIR/check-version-upgrade.py" verify "$root" "$window"
[[ "$(<"$extension_data_file")" == private-extension-state-survived-real-package-upgrade ]] || fail 'reinstalled launcher/worker lost the private extension value'
stop_launcher_gracefully

if ! revoke_output="$(search_cli owner grant revoke --instance-dir "$instance" "$grant_digest" 2>"$search_root/grant-revoke.err")"; then
  cat "$search_root/grant-revoke.err" >&2
  fail 'could not revoke the retained search consumer grant after upgrade/reinstall acceptance'
fi
[[ "$revoke_output" == *'state=revoked'* ]] || fail 'search grant revoke did not confirm state=revoked'
if search_cli search "${consumer_args[@]}" --limit 1 "$primary_phrase" >"$search_root/final-revoked.stdout" 2>"$search_root/final-revoked.stderr"; then
  fail 'revoked search consumer unexpectedly searched the approved passage'
fi
[[ ! -s "$search_root/final-revoked.stdout" && "$(<"$search_root/final-revoked.stderr")" == *Unauthorized* ]] || {
  cat "$search_root/final-revoked.stderr" >&2
  fail 'revoked search consumer did not receive typed Unauthorized'
}
printf '%s\n' "VERSION_UPGRADE_SEARCH_GRANT_REVOKED digest=$grant_digest denial=Unauthorized"
stop_search_gracefully

start_launcher
window="$launcher_window"
timeout --kill-after=1s 45s python3 "$SCRIPT_DIR/check-version-upgrade.py" revoke "$root" "$window"
[[ "$(<"$extension_data_file")" == private-extension-state-survived-real-package-upgrade ]] || fail 'revoking the extension permission unexpectedly removed its private data'
stop_launcher_gracefully
printf '%s\n' "VERSION_UPGRADE_ACCEPTANCE_PASSED old_upstream=${old_upstreams[0]} new_upstream=${new_upstreams[0]} source_old=$OLD_SOURCE_REVISION source_new=$new_source_revision persistent_search_evidence_id=$primary_evidence_id retained_xdg_state_through_reinstall=true revoked_grants=search,extension private_xvfb=true private_dbus=true bwrap_worker=true"

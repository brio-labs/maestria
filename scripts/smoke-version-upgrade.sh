#!/usr/bin/env bash
set -euo pipefail

# These are the immutable identities of the pinned historical 0.0.0 artifacts.
# They apply only to the legacy input set; current artifacts use Sillage IDs.
LEGACY_PACKAGE_NAMES=(
  io-github-briolabs-maestria-launcher
  io-github-briolabs-maestria-search
  io-github-briolabs-maestria-extension-worker
)
CURRENT_PACKAGE_NAMES=(
  io-github-briolabs-sillage-launcher
  io-github-briolabs-sillage-search
  io-github-briolabs-sillage-extension-worker
)
LEGACY_BINARY_PATHS=(
  /usr/bin/maestria-launcher
  /usr/bin/maestria-search
  /usr/bin/maestria-extension-worker
)
CURRENT_BINARY_PATHS=(
  /usr/bin/sillage-launcher
  /usr/bin/sillage-search
  /usr/bin/sillage-extension-worker
)
LEGACY_APP_ID=io.github.briolabs.Maestria.Launcher
CURRENT_APP_ID=io.github.briolabs.Sillage.Launcher
LEGACY_SOURCE_REVISION=985a8368458d7267a4b150ae52702f90fbeedbcd
LEGACY_ARTIFACT_RUN=36455626998
LEGACY_SOURCE_UPSTREAM=0.0.0
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

fail() { echo "cross-name package smoke failed: $*" >&2; exit 1; }

activate_suite() {
  local suite="$1"
  case "$suite" in
    legacy)
      active_suite=legacy-maestria
      package_names=("${LEGACY_PACKAGE_NAMES[@]}")
      binary_paths=("${LEGACY_BINARY_PATHS[@]}")
      ;;
    current)
      active_suite=current-sillage
      package_names=("${CURRENT_PACKAGE_NAMES[@]}")
      binary_paths=("${CURRENT_BINARY_PATHS[@]}")
      ;;
    *)
      fail "unknown package suite $suite"
      ;;
  esac
  select_search_state "$suite"
}

usage() {
  echo "usage: smoke-version-upgrade.sh LEGACY_MAESTRIA_LAUNCHER_DIR LEGACY_MAESTRIA_SEARCH_DIR LEGACY_MAESTRIA_WORKER_DIR CURRENT_SILLAGE_LAUNCHER_DIR CURRENT_SILLAGE_SEARCH_DIR CURRENT_SILLAGE_WORKER_DIR" >&2
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
all_product_package_names=("${LEGACY_PACKAGE_NAMES[@]}" "${CURRENT_PACKAGE_NAMES[@]}")

package_versions=()
package_sha256=()
package_upstreams=()
for index in 0 1 2; do
  old_index=$index
  new_index=$((index + 3))
  for suite in legacy current; do
    if [[ "$suite" == legacy ]]; then
      package_index="$old_index"
      expected_package="${LEGACY_PACKAGE_NAMES[$index]}"
    else
      package_index="$new_index"
      expected_package="${CURRENT_PACKAGE_NAMES[$index]}"
    fi
    package="${package_paths[$package_index]}"
    actual_package="$(dpkg-deb --field "$package" Package)" || fail "cannot read package name from $package"
    [[ "$actual_package" == "$expected_package" ]] || fail "$suite artifact must retain package identity $expected_package, found $actual_package"
    architecture="$(dpkg-deb --field "$package" Architecture)" || fail "cannot read architecture from $package"
    [[ "$architecture" == amd64 ]] || fail "expected amd64 package $package, found $architecture"
    version="$(dpkg-deb --field "$package" Version)" || fail "cannot read version from $package"
    [[ -n "$version" ]] || fail "package version is empty in $package"
    dependencies="$(dpkg-deb --field "$package" Depends 2>/dev/null || true) $(dpkg-deb --field "$package" Pre-Depends 2>/dev/null || true) $(dpkg-deb --field "$package" Recommends 2>/dev/null || true) $(dpkg-deb --field "$package" Suggests 2>/dev/null || true)"
    for other_package in "${all_product_package_names[@]}"; do
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
  legacy_package="${LEGACY_PACKAGE_NAMES[$index]}"
  current_package="${CURRENT_PACKAGE_NAMES[$index]}"
  [[ "$old_upstream" == "$LEGACY_SOURCE_UPSTREAM" ]] || fail "$legacy_package legacy upstream version must be $LEGACY_SOURCE_UPSTREAM, found $old_upstream"
  [[ "$new_upstream" != "$old_upstream" ]] || fail "$current_package legacy and current upstream versions are identical: $old_upstream"
  dpkg --compare-versions "$old_upstream" lt "$new_upstream" || fail "$current_package upstream version is not newer: legacy=$old_upstream current=$new_upstream"
  [[ "${package_sha256[$((2 * index))]}" != "${package_sha256[$((2 * index + 1))]}" ]] || fail "$legacy_package and $current_package Debian artifact SHA-256 are identical"
  if [[ ${7:-} == --inside ]]; then
    printf 'CROSSNAME_INSTALL_ARTIFACT legacy_package=%s legacy_version=%s legacy_sha256=%s current_package=%s current_version=%s current_sha256=%s\n' \
      "$legacy_package" "$old_version" "${package_sha256[$((2 * index))]}" "$current_package" "$new_version" "${package_sha256[$((2 * index + 1))]}"
  fi
done

old_versions=("${package_versions[0]}" "${package_versions[2]}" "${package_versions[4]}")
new_versions=("${package_versions[1]}" "${package_versions[3]}" "${package_versions[5]}")
old_upstreams=("${package_upstreams[0]}" "${package_upstreams[2]}" "${package_upstreams[4]}")
new_upstreams=("${package_upstreams[1]}" "${package_upstreams[3]}" "${package_upstreams[5]}")
[[ "${old_versions[0]}" == "${old_versions[1]}" && "${old_versions[1]}" == "${old_versions[2]}" ]] || fail 'old launcher, search, and worker artifacts do not share one upstream package version'
[[ "${new_versions[0]}" == "${new_versions[1]}" && "${new_versions[1]}" == "${new_versions[2]}" ]] || fail 'new launcher, search, and worker artifacts do not share one upstream package version'
[[ "${old_upstreams[0]}" == "$LEGACY_SOURCE_UPSTREAM" && "${new_upstreams[0]}" == "${new_upstreams[1]}" && "${new_upstreams[1]}" == "${new_upstreams[2]}" ]] || fail 'legacy and current artifact sets have inconsistent upstream product versions'

repo_root="$(realpath -- "$SCRIPT_DIR/..")"
checkout_revision="$(git -C "$repo_root" rev-parse --verify HEAD 2>/dev/null)" || fail 'cannot identify the checked-out committed source revision'
new_source_revision="${NEW_SOURCE_REVISION:-${GITHUB_SHA:-$checkout_revision}}"
[[ "$LEGACY_SOURCE_REVISION" =~ ^[0-9a-f]{40,64}$ && "$new_source_revision" =~ ^[0-9a-f]{40,64}$ ]] || fail 'source provenance must use full lowercase Git commit hashes'
[[ "$new_source_revision" == "$checkout_revision" ]] || fail "new artifact source revision $new_source_revision does not match checked-out source $checkout_revision"
[[ "$LEGACY_SOURCE_REVISION" != "$new_source_revision" ]] || fail 'legacy and current artifacts claim the same committed source revision'
if [[ -n "${GITHUB_SHA:-}" ]]; then
  [[ "$GITHUB_SHA" == "$checkout_revision" ]] || fail "GitHub artifact source $GITHUB_SHA differs from checked-out source $checkout_revision"
fi
git -C "$repo_root" cat-file -e "${LEGACY_SOURCE_REVISION}^{commit}" 2>/dev/null || fail 'legacy source commit is unavailable locally; fetch the release baseline before running this smoke'
product_source_diff="$(git -C "$repo_root" diff --name-only "$LEGACY_SOURCE_REVISION" "$new_source_revision" -- crates src launcher/src extension-sdk/src)" || fail 'cannot compare legacy and current committed product sources'
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
  printf 'CROSSNAME_INSTALL_PROVENANCE legacy_run=%s legacy_source_revision=%s current_source_revision=%s\n' \
    "$LEGACY_ARTIFACT_RUN" "$LEGACY_SOURCE_REVISION" "$new_source_revision"
fi
if [[ ${7:-} == --inside ]]; then
  [[ $# -eq 8 && -d "$8" ]] || usage
  root="$(realpath -- "$8")"
else
  for tool in xvfb-run dbus-run-session jwm xdotool xclip xprop gdbus; do
    command -v "$tool" >/dev/null 2>&1 || fail "required private desktop tool is unavailable: $tool"
  done
  umask 077
  root="$(mktemp -d "${TMPDIR:-/tmp}/sillage-version-upgrade.XXXXXX")"
  chmod 700 "$root"
  cleanup_outer() {
    # Installed extension bundles seal package directories read-only; after the
    # private launcher exits, restore owner write access only inside this root.
    local app_id sealed
    for app_id in "$LEGACY_APP_ID" "$CURRENT_APP_ID"; do
      sealed="$root/data/$app_id/extensions/packages"
      if [[ -d "$sealed" && ! -L "$sealed" ]]; then chmod -R u+w -- "$sealed"; fi
    done
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

shutdown_deadline() {
  python3 -c 'import time; print(time.monotonic_ns() + 5_000_000_000)'
}
await_process_shutdown() {
  python3 - "$@" <<'PY'
import os
import select
import subprocess
import sys
import time

pid, deadline = map(int, sys.argv[1:3])
started = deadline - 5_000_000_000
quit_process = None
descriptors = []
try:
    if len(sys.argv) == 4:
        quit_process = subprocess.Popen([sys.argv[3], "--quit"], stdout=subprocess.DEVNULL)
    for target in [pid] + ([quit_process.pid] if quit_process is not None else []):
        try:
            descriptor = os.pidfd_open(target)
        except ProcessLookupError:
            continue
        descriptors.append(descriptor)
        remaining = max(0, (deadline - time.monotonic_ns()) / 1e9)
        if not select.select([descriptor], [], [], remaining)[0]:
            raise TimeoutError("five-second process absence gate failed")
    if quit_process is not None and quit_process.wait() != 0:
        raise RuntimeError("resident launcher quit request failed")
    elapsed = time.monotonic_ns() - started
    if time.monotonic_ns() > deadline:
        raise TimeoutError("five-second shutdown deadline exceeded")
    print(elapsed)
except (TimeoutError, RuntimeError) as error:
    if quit_process is not None and quit_process.poll() is None:
        quit_process.terminate()
    print(str(error), file=sys.stderr)
    raise SystemExit(1)
finally:
    for descriptor in descriptors:
        os.close(descriptor)
PY
}
stop_search_best_effort() {
  [[ -n "$search_pid" ]] || return 0
  kill -TERM "$search_pid" 2>/dev/null || true
  await_process_shutdown "$search_pid" "$1" >/dev/null || return 1
  wait "$search_pid" 2>/dev/null || true
  search_pid=
}
stop_launcher_best_effort() {
  [[ -n "$launcher_pid" ]] || return 0
  kill -TERM -- "-$launcher_pid" 2>/dev/null || kill -TERM "$launcher_pid" 2>/dev/null || true
  await_process_shutdown "$launcher_pid" "$1" >/dev/null || return 1
  wait "$launcher_pid" 2>/dev/null || true
  launcher_pid=
}
cleanup_inner() {
  local status=$? deadline shutdown_failed=false
  trap - EXIT INT TERM
  deadline="$(shutdown_deadline)"
  stop_launcher_best_effort "$deadline" || shutdown_failed=true
  stop_search_best_effort "$deadline" || shutdown_failed=true
  for pid in "$wm_pid" "$registry_pid"; do
    if [[ -n "$pid" ]]; then
      kill -TERM "$pid" 2>/dev/null || true
      if await_process_shutdown "$pid" "$deadline" >/dev/null; then
        wait "$pid" 2>/dev/null || true
      else
        shutdown_failed=true
      fi
    fi
  done
  printf 'VERSION_UPGRADE_CLEANUP_SHUTDOWN five_second_absence_gate=%s force_kill_sent=false\n' "$([[ "$shutdown_failed" == false ]] && printf true || printf false)"
  if [[ "$shutdown_failed" == true && "$status" -eq 0 ]]; then status=125; fi
  [[ ! -s "$launcher_log" ]] || cat "$launcher_log" >&2
  [[ ! -s "$search_log" ]] || cat "$search_log" >&2
  exit "$status"
}
trap cleanup_inner EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

extension_id=dev.sillage.version-upgrade
profile_state_value=private-extension-profile-state-retained-on-reinstall
legacy_config_file="$root/config/$LEGACY_APP_ID/launcher.toml"
current_config_file="$root/config/$CURRENT_APP_ID/launcher.toml"
legacy_extension_store="$root/data/$LEGACY_APP_ID/extensions"
current_extension_store="$root/data/$CURRENT_APP_ID/extensions"
mkdir -p "$root/config/$LEGACY_APP_ID" "$root/package/dist" "$root/search"
cat > "$legacy_config_file" <<'TOML'
schemaVersion = 1
shortcut = "Control+Space"
shortcutSetup = "deferred"
reduceMotion = true
TOML
chmod 600 "$legacy_config_file"

cat > "$root/package/manifest.json" <<'JSON'
{"apiVersion":1,"id":"dev.sillage.version-upgrade","name":"Version Upgrade Broker","version":"1.0.0","entrypoints":[{"id":"main","file":"dist/main.js"}],"commands":[{"id":"state-check","title":"Version upgrade state","entrypointId":"main"}],"permissions":[{"type":"copy","formats":["text"]},{"type":"storage","scope":"extension"}]}
JSON
cat > "$root/package/dist/main.js" <<'JS'
const stateKey = "upgrade-proof";
const stateValue = "private-extension-profile-state-retained-on-reinstall";
const clipboardValue = "private-xvfb-crossname-profile-copy-proof";

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
  if [[ "$set" == old ]]; then
    versions=("${old_versions[@]}")
    activate_suite legacy
  elif [[ "$set" == new ]]; then
    versions=("${new_versions[@]}")
    activate_suite current
  else
    fail "unknown installed package set $set"
  fi
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
    printf 'CROSSNAME_INSTALL_PACKAGE_VERIFIED set=%s suite=%s package=%s version=%s binary=%s binary_sha256=%s owner=%s\n' \
      "$set" "$active_suite" "${package_names[$index]}" "$expected_version" "$binary" "$binary_hash" "$owner"
    if [[ "$set" == new ]]; then
      old_hash="${old_binary_sha256[$index]}"
      [[ "$binary_hash" != "$old_hash" ]] || fail "${CURRENT_PACKAGE_NAMES[$index]} canonical executable bytes are identical to pinned legacy ${LEGACY_PACKAGE_NAMES[$index]}"
      if [[ -n "${new_binary_sha256[$index]:-}" ]]; then
        [[ "$binary_hash" == "${new_binary_sha256[$index]}" ]] || fail "${CURRENT_PACKAGE_NAMES[$index]} reinstall changed the canonical package executable bytes"
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
  setsid "${binary_paths[0]}" --activate >"$launcher_log" 2>&1 & launcher_pid=$!
  launcher_window="$(private_launcher_window)"
  timeout --kill-after=1s 5s xdotool windowactivate --sync "$launcher_window"
  printf 'VERSION_UPGRADE_LAUNCHER_STARTED suite=%s path=%s pid=%s window=%s\n' "$active_suite" "${binary_paths[0]}" "$launcher_pid" "$launcher_window"
}
stop_launcher_gracefully() {
  [[ -n "$launcher_pid" ]] || return 0
  local status=0 deadline elapsed_ns
  deadline="$(shutdown_deadline)"
  elapsed_ns="$(await_process_shutdown "$launcher_pid" "$deadline" "${binary_paths[0]}")" || { cat "$launcher_log" >&2; fail 'resident launcher did not stop gracefully within five seconds after --quit'; }
  wait "$launcher_pid" || status=$?
  [[ "$status" -eq 0 ]] || fail "resident launcher exited with status $status after --quit"
  launcher_pid=
  printf 'VERSION_UPGRADE_LAUNCHER_STOPPED_GRACEFULLY exit_status=%s shutdown_elapsed_ns=%s five_second_absence_gate=true force_kill_sent=false\n' "$status" "$elapsed_ns"
}

legacy_search_root="$root/search/legacy"
current_search_root="$root/search/current"
select_search_state() {
  local profile="$1"
  case "$profile" in
    legacy)
      search_profile=legacy
      search_root="$legacy_search_root"
      primary_phrase='The silver kestrel carries an upgrade-proof compass through quiet rain'
      ungranted_phrase='A violet meteor hides the ungranted Maestria upgrade-only archive'
      ;;
    current)
      search_profile=current
      search_root="$current_search_root"
      primary_phrase='The amber heron preserves an isolated Sillage search namespace through winter drizzle'
      ungranted_phrase='A copper comet marks the ungranted Sillage upgrade-only archive'
      ;;
    *)
      fail "unknown search namespace $profile"
      ;;
  esac
  instance="$search_root/instance"
  approved_root="$search_root/approved-root"
  ungranted_root="$search_root/ungranted-root"
  credential_file="$search_root/consumer.credential"
  socket_path="$instance/system/daemon.sock"
  fixture="$approved_root/upgrade-passage.md"
  ungranted_fixture="$ungranted_root/private-note.md"
}

search_cli() { timeout --kill-after=1s 12s "${binary_paths[1]}" "$@"; }

setup_search_instance() {
  mkdir -m 700 -p "$search_root"
  mkdir -m 700 "$approved_root" "$ungranted_root" "$instance"
  printf '%s\n' "# Durable $search_profile version upgrade passage" "$primary_phrase." >"$fixture"
  printf '%s\n' "# Unapproved private $search_profile root" "$ungranted_phrase." >"$ungranted_fixture"
  fixture="$(realpath -- "$fixture")"
  approved_root="$(realpath -- "$approved_root")"
  ungranted_root="$(realpath -- "$ungranted_root")"
  if ! search_cli init --instance-dir "$instance" --read-root "$approved_root" >"$search_root/init.out" 2>"$search_root/init.err"; then
    cat "$search_root/init.err" >&2
    fail "installed $search_profile search binary could not initialize its private search instance"
  fi
  provider_realm="$(sed -n 's/^realm_id=//p' "$instance/manifest.txt")"
  [[ "$provider_realm" =~ ^[0-9a-f]{64}$ ]] || fail "$search_profile search instance has no valid provider realm"
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
  [[ "$grant_output" == *"consumer_realm=$consumer_realm"* && "$grant_output" == *'state=active'* ]] || fail "$search_profile external grant output did not confirm its consumer grant"
  [[ "$grant_output" == *"allowed_roots=[\"$approved_root\"]"* ]] || fail "$search_profile consumer grant did not freeze exactly its approved private root"
  grant_digest="$(sed -n 's/^grant_token_digest=//p' <<<"$grant_output")"
  [[ "$grant_digest" =~ ^[0-9a-f]{64}$ && -s "$credential_file" ]] || fail "$search_profile search grant lacks its digest or credential"
  [[ "$(stat -c '%a' "$credential_file")" == 600 ]] || fail "$search_profile search credential is not mode 0600"
  consumer_args=(--socket-path "$socket_path" --consumer-realm "$consumer_realm" --credential-file "$credential_file")
  ungranted_args=(--socket-path "$socket_path" --consumer-realm "$ungranted_realm" --credential-file "$credential_file")
  printf 'VERSION_UPGRADE_SEARCH_NAMESPACE profile=%s instance=%s grant=active credential_mode=0600 approved_root=%s ungranted_root=%s\n' \
    "$search_profile" "$instance" "$approved_root" "$ungranted_root"
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
  "${binary_paths[1]}" start --instance-dir "$instance" >"$search_log" 2>&1 & search_pid=$!
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
  printf 'VERSION_UPGRADE_SEARCH_DAEMON_STARTED suite=%s binary=%s pid=%s socket=%s\n' "$active_suite" "${binary_paths[1]}" "$search_pid" "$socket_path"
  if [[ -s "$credential_file" ]]; then wait_for_search_index; fi
}

stop_search_gracefully() {
  [[ -n "$search_pid" ]] || return 0
  local status=0 deadline elapsed_ns
  deadline="$(shutdown_deadline)"
  kill -INT "$search_pid" 2>/dev/null || fail 'cannot request graceful search daemon shutdown'
  elapsed_ns="$(await_process_shutdown "$search_pid" "$deadline")" || fail 'search daemon failed to stop gracefully within five seconds (SIGKILL is not accepted)'
  wait "$search_pid" || status=$?
  [[ "$status" -eq 0 ]] || fail "search daemon exited with status $status after SIGINT"
  search_pid=
  [[ ! -e "$socket_path" ]] || fail 'gracefully stopped search daemon left its private socket behind'
  printf 'VERSION_UPGRADE_SEARCH_DAEMON_STOPPED_GRACEFULLY exit_status=%s shutdown_elapsed_ns=%s five_second_absence_gate=true force_kill_sent=false\n' "$status" "$elapsed_ns"
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
root_metadata = root.lstat()
if not stat.S_ISDIR(root_metadata.st_mode):
    raise SystemExit(f"private persistent state directory is missing or not a real directory: {root}")
digest = hashlib.sha256()
digest.update(b".")
digest.update(stat.S_IMODE(root_metadata.st_mode).to_bytes(4, "big"))
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

hash_private_file() {
  python3 - "$1" <<'PY'
import hashlib
import stat
import sys
from pathlib import Path
path = Path(sys.argv[1])
metadata = path.lstat()
if not stat.S_ISREG(metadata.st_mode):
    raise SystemExit(f"private persistent state file is missing or not a regular file: {path}")
content = path.read_bytes()
digest = hashlib.sha256()
digest.update(stat.S_IMODE(metadata.st_mode).to_bytes(4, "big"))
digest.update(len(content).to_bytes(8, "big"))
digest.update(content)
print(digest.hexdigest())
PY
}
state_key_hex="$(printf '%s' upgrade-proof | od -An -tx1 | tr -d ' \n')"
legacy_extension_data_file="$legacy_extension_store/data/$extension_id/$extension_id/$state_key_hex"
current_extension_data_file="$current_extension_store/data/$extension_id/$extension_id/$state_key_hex"

select_profile_state() {
  local profile="$1"
  case "$profile" in
    legacy)
      profile_config_dir="$root/config/$LEGACY_APP_ID"
      profile_config_file="$legacy_config_file"
      profile_extension_store="$legacy_extension_store"
      profile_extension_data_file="$legacy_extension_data_file"
      ;;
    current)
      profile_config_dir="$root/config/$CURRENT_APP_ID"
      profile_config_file="$current_config_file"
      profile_extension_store="$current_extension_store"
      profile_extension_data_file="$current_extension_data_file"
      ;;
    *)
      fail "unknown XDG profile $profile"
      ;;
  esac
}

snapshot_private_state() {
  local label="$1" profile="$2"
  select_search_state "$profile"
  select_profile_state "$profile"
  [[ -s "$profile_config_file" ]] || fail "$label $profile launcher settings file is missing"
  [[ "$(stat -c '%a' "$profile_config_file")" == 600 ]] || fail "$label $profile launcher settings file is not private mode 0600"
  [[ -s "$profile_extension_data_file" && "$(<"$profile_extension_data_file")" == "$profile_state_value" ]] || fail "$label $profile extension private storage value is missing or changed"
  [[ "$(stat -c '%a' "$profile_extension_data_file")" == 600 ]] || fail "$label $profile extension state file is not mode 0600"
  [[ -s "$credential_file" && "$(stat -c '%a' "$credential_file")" == 600 ]] || fail "$label $profile search credential is missing or not mode 0600"
  config_sha="$(hash_tree "$profile_config_dir")"
  extension_sha="$(hash_tree "$profile_extension_store")"
  search_sha="$(hash_tree "$instance")"
  credential_sha="$(hash_private_file "$credential_file")"
  printf 'CROSSNAME_PROFILE_STATE profile=%s phase=%s launcher_settings=verified extension_permissions_and_data=verified search_instance=verified consumer_credential=verified\n' \
    "$profile" "$label"
}

assert_profile_state_matches() {
  local label="$1" profile="$2" expected_config="$3" expected_extension="$4"
  select_profile_state "$profile"
  [[ "$(hash_tree "$profile_config_dir")" == "$expected_config" ]] || fail "$label modified $profile launcher settings bytes or modes"
  [[ "$(hash_tree "$profile_extension_store")" == "$expected_extension" ]] || fail "$label modified $profile extension permissions/package/data bytes or modes"
  [[ "$(<"$profile_extension_data_file")" == "$profile_state_value" ]] || fail "$label changed the $profile extension profile value"
  printf 'CROSSNAME_PROFILE_STATE_UNCHANGED profile=%s phase=%s launcher_settings=verified extension_store=verified\n' \
    "$profile" "$label"
}

assert_private_state_matches() {
  local label="$1" profile="$2" expected_config="$3" expected_extension="$4" expected_search="$5" expected_credential="$6"
  select_search_state "$profile"
  select_profile_state "$profile"
  [[ "$(hash_tree "$profile_config_dir")" == "$expected_config" ]] || fail "$label modified $profile launcher settings bytes or modes"
  [[ "$(hash_tree "$profile_extension_store")" == "$expected_extension" ]] || fail "$label modified $profile extension permissions/package/data bytes or modes"
  [[ "$(hash_tree "$instance")" == "$expected_search" ]] || fail "$label modified the $profile explicit search instance/grant bytes or modes"
  [[ "$(hash_private_file "$credential_file")" == "$expected_credential" ]] || fail "$label modified the $profile consumer credential bytes or mode"
  [[ "$(<"$profile_extension_data_file")" == "$profile_state_value" ]] || fail "$label changed the $profile extension profile value"
  printf 'CROSSNAME_PROFILE_STATE_UNCHANGED profile=%s phase=%s launcher_settings=verified extension_store=verified search_instance=verified consumer_credential=verified\n' \
    "$profile" "$label"
}

for package in "${all_product_package_names[@]}"; do
  status="$(dpkg-query --show --showformat='${Status}' "$package" 2>/dev/null || true)"
  [[ "$status" != 'install ok installed' ]] || fail "runner already has $package installed; refusing a pre-existing package state"
done

run_apt 'install the pinned historical Maestria 0.0.0 artifacts' install --yes --no-install-recommends "${package_paths[0]}" "${package_paths[1]}" "${package_paths[2]}"
old_binary_sha256=() new_binary_sha256=()
verify_installed_set old
activate_suite legacy
[[ ! -e "$legacy_extension_data_file" ]] || fail 'legacy extension profile was not fresh before first setup'

setup_search_instance
start_search_daemon
create_search_grant
legacy_consumer_realm="$consumer_realm"
legacy_credential_file="$credential_file"
wait_for_search_index
verify_search_behavior legacy-maestria ""
legacy_evidence_id="$primary_evidence_id"

start_launcher
window="$launcher_window"
timeout --kill-after=1s 45s python3 "$SCRIPT_DIR/check-version-upgrade.py" prepare legacy "$root" "$window"
[[ -s "$legacy_extension_data_file" ]] || fail 'pinned Maestria extension worker did not create its private XDG storage entry'
[[ "$(<"$legacy_extension_data_file")" == "$profile_state_value" ]] || fail 'legacy extension worker stored an unexpected private profile value'
[[ "$(stat -c '%a' "$legacy_extension_data_file")" == 600 ]] || fail 'legacy extension worker data is not mode 0600'
stop_launcher_gracefully
stop_search_gracefully
snapshot_private_state before-sillage-install legacy
legacy_saved_config_sha="$config_sha"
legacy_saved_extension_sha="$extension_sha"
legacy_saved_search_sha="$search_sha"
legacy_saved_credential_sha="$credential_sha"

run_apt 'install canonical Sillage artifacts alongside the pinned Maestria packages; do not migrate profiles' install --yes --no-install-recommends "${package_paths[3]}" "${package_paths[4]}" "${package_paths[5]}"
verify_installed_set new
verify_installed_set old
assert_private_state_matches after-sillage-install-before-sillage-launch legacy "$legacy_saved_config_sha" "$legacy_saved_extension_sha" "$legacy_saved_search_sha" "$legacy_saved_credential_sha"
[[ ! -e "$current_config_file" && ! -e "$current_extension_store" ]] || fail 'Sillage config or extension profile exists before first Sillage setup; cross-name migration is unsupported'
printf 'CROSSNAME_PROFILE_ISOLATION_VERIFIED legacy_app_id=%s current_app_id=%s legacy_config=%s current_config_absent=true current_extension_state_absent=true\n' \
  "$LEGACY_APP_ID" "$CURRENT_APP_ID" "$legacy_config_file"

activate_suite current
setup_search_instance
start_search_daemon
create_search_grant
wait_for_search_index
if search_cli search --socket-path "$socket_path" --consumer-realm "$legacy_consumer_realm" \
  --credential-file "$legacy_credential_file" --limit 1 "$primary_phrase" \
  >"$search_root/legacy-credential-on-current.stdout" 2>"$search_root/legacy-credential-on-current.stderr"; then
  fail 'legacy Maestria consumer credential unexpectedly authorized against the separate Sillage instance'
fi
[[ ! -s "$search_root/legacy-credential-on-current.stdout" && "$(<"$search_root/legacy-credential-on-current.stderr")" == *Unauthorized* ]] || {
  cat "$search_root/legacy-credential-on-current.stderr" >&2
  fail 'legacy Maestria consumer credential did not receive typed Unauthorized against the Sillage instance'
}
printf 'VERSION_UPGRADE_SEARCH_NAMESPACE_ISOLATION legacy_credential_on_current=Unauthorized\n'
verify_search_behavior sillage-explicit-private-instance ""
current_evidence_id="$primary_evidence_id"
stop_search_gracefully

[[ ! -e "$current_config_file" && ! -e "$current_extension_store" ]] || fail 'Sillage config or extension profile appeared before its first launcher run'
printf 'CROSSNAME_PROFILE_FRESH_DEFAULTS_START current_config_absent=true current_extension_state_absent=true\n'
start_launcher
window="$launcher_window"
timeout --kill-after=1s 45s python3 "$SCRIPT_DIR/check-version-upgrade.py" defaults current "$root" "$window"
stop_launcher_gracefully
[[ -s "$current_config_file" ]] || fail 'Sillage preferences UI did not persist its configured settings after fresh-defaults verification'
[[ "$(stat -c '%a' "$current_config_file")" == 600 ]] || fail 'Sillage preferences UI did not persist mode 0600 settings'
printf 'CROSSNAME_PROFILE_FRESH_DEFAULTS_TRANSITION config_persisted=true mode=0600 configured_via=preferences_ui\n'
assert_private_state_matches after-sillage-defaults-and-configuration legacy "$legacy_saved_config_sha" "$legacy_saved_extension_sha" "$legacy_saved_search_sha" "$legacy_saved_credential_sha"

start_launcher
window="$launcher_window"
timeout --kill-after=1s 45s python3 "$SCRIPT_DIR/check-version-upgrade.py" prepare current "$root" "$window"
[[ -s "$current_extension_data_file" ]] || fail 'Sillage extension worker did not create its private XDG storage entry'
[[ "$(<"$current_extension_data_file")" == "$profile_state_value" ]] || fail 'Sillage extension worker stored an unexpected private profile value'
[[ "$(stat -c '%a' "$current_extension_data_file")" == 600 ]] || fail 'Sillage extension data is not mode 0600'
stop_launcher_gracefully
snapshot_private_state before-sillage-remove-reinstall current
current_saved_config_sha="$config_sha"
current_saved_extension_sha="$extension_sha"
current_saved_search_sha="$search_sha"
current_saved_credential_sha="$credential_sha"
assert_private_state_matches after-sillage-profile-creation legacy "$legacy_saved_config_sha" "$legacy_saved_extension_sha" "$legacy_saved_search_sha" "$legacy_saved_credential_sha"

run_apt 'remove only the canonical Sillage package suite, leaving the distinct Maestria suite installed' remove --yes --no-install-recommends "${CURRENT_PACKAGE_NAMES[@]}"
for binary in "${CURRENT_BINARY_PATHS[@]}"; do
  [[ ! -e "$binary" ]] || fail "Sillage package removal left installed executable $binary"
done
for package in "${CURRENT_PACKAGE_NAMES[@]}"; do
  status="$(dpkg-query --show --showformat='${Status}' "$package" 2>/dev/null || true)"
  [[ "$status" != 'install ok installed' ]] || fail "apt remove left current Sillage package $package installed"
done
verify_installed_set old
assert_private_state_matches after-sillage-remove current "$current_saved_config_sha" "$current_saved_extension_sha" "$current_saved_search_sha" "$current_saved_credential_sha"
assert_private_state_matches after-sillage-remove legacy "$legacy_saved_config_sha" "$legacy_saved_extension_sha" "$legacy_saved_search_sha" "$legacy_saved_credential_sha"

run_apt 'reinstall the exact canonical Sillage artifacts' install --yes --no-install-recommends "${package_paths[3]}" "${package_paths[4]}" "${package_paths[5]}"
verify_installed_set new
verify_installed_set old
assert_private_state_matches after-sillage-reinstall current "$current_saved_config_sha" "$current_saved_extension_sha" "$current_saved_search_sha" "$current_saved_credential_sha"
assert_private_state_matches after-sillage-reinstall legacy "$legacy_saved_config_sha" "$legacy_saved_extension_sha" "$legacy_saved_search_sha" "$legacy_saved_credential_sha"

activate_suite current
start_search_daemon
verify_search_behavior sillage-reinstall "$current_evidence_id"
start_launcher
window="$launcher_window"
timeout --kill-after=1s 45s python3 "$SCRIPT_DIR/check-version-upgrade.py" verify current "$root" "$window"
[[ "$(<"$current_extension_data_file")" == "$profile_state_value" ]] || fail 'reinstalled Sillage launcher/worker lost its private extension profile value'
stop_launcher_gracefully

if ! revoke_output="$(search_cli owner grant revoke --instance-dir "$instance" "$grant_digest" 2>"$search_root/grant-revoke.err")"; then
  cat "$search_root/grant-revoke.err" >&2
  fail 'could not revoke the retained search consumer grant after Sillage remove/reinstall acceptance'
fi
[[ "$revoke_output" == *'state=revoked'* ]] || fail 'search grant revoke did not confirm state=revoked'
if search_cli search "${consumer_args[@]}" --limit 1 "$primary_phrase" >"$search_root/final-revoked.stdout" 2>"$search_root/final-revoked.stderr"; then
  fail 'revoked search consumer unexpectedly searched the approved passage'
fi
[[ ! -s "$search_root/final-revoked.stdout" && "$(<"$search_root/final-revoked.stderr")" == *Unauthorized* ]] || {
  cat "$search_root/final-revoked.stderr" >&2
  fail 'revoked search consumer did not receive typed Unauthorized'
}
printf '%s\n' "CROSSNAME_SEARCH_GRANT_REVOKED profile=current denial=Unauthorized"
stop_search_gracefully

start_launcher
window="$launcher_window"
timeout --kill-after=1s 45s python3 "$SCRIPT_DIR/check-version-upgrade.py" revoke current "$root" "$window"
[[ "$(<"$current_extension_data_file")" == "$profile_state_value" ]] || fail 'revoking the Sillage extension permission unexpectedly removed its private data'
stop_launcher_gracefully
assert_private_state_matches final-current-revocation legacy "$legacy_saved_config_sha" "$legacy_saved_extension_sha" "$legacy_saved_search_sha" "$legacy_saved_credential_sha"
printf '%s\n' "CROSSNAME_INSTALL_ACCEPTANCE_PASSED legacy_run=$LEGACY_ARTIFACT_RUN legacy_upstream=${old_upstreams[0]} current_upstream=${new_upstreams[0]} legacy_source=$LEGACY_SOURCE_REVISION current_source=$new_source_revision legacy_launcher_extension_search_and_credential_unchanged=true separate_legacy_and_current_search_instances=true current_profile_retained_through_remove_reinstall=true legacy_search_evidence_id=$legacy_evidence_id current_search_evidence_id=$current_evidence_id revoked_grants=current-search,current-extension private_xvfb=true private_dbus=true bwrap_worker=true"

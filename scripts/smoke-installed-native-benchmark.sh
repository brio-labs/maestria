#!/usr/bin/env bash
set -euo pipefail

LAUNCHER_PACKAGE=io-github-briolabs-maestria-launcher
SEARCH_PACKAGE=io-github-briolabs-maestria-search
LAUNCHER_BINARY=/usr/bin/maestria-launcher
SEARCH_BINARY=/usr/bin/maestria-search
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
DRIVER="$SCRIPT_DIR/smoke-installed-native-benchmark.py"

fail() {
  printf 'installed native benchmark failed: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'USAGE'
Usage: smoke-installed-native-benchmark.sh LAUNCHER_PACKAGE_DIRECTORY SEARCH_PACKAGE_DIRECTORY OUTPUT_DIRECTORY

Requires the exact launcher and search Debian packages to be installed already.
Runs only in private XDG, D-Bus, Xvfb, JWM, and AT-SPI state; starts the
installed search daemon against a deterministic 10,000-file approved root.
USAGE
}

if [[ $# -eq 1 && ( "$1" == -h || "$1" == --help ) ]]; then
  usage
  exit 0
fi

private_session=false
if [[ $# -eq 5 && "$1" == --private-session ]]; then
  private_session=true
  private_root="$(realpath "$2")"
  launcher_dir="$(realpath "$3")"
  search_dir="$(realpath "$4")"
  output_dir="$(realpath "$5")"
else
  [[ $# -eq 3 ]] || { usage >&2; exit 2; }
  launcher_dir="$(realpath "$1")"
  search_dir="$(realpath "$2")"
  mkdir -p -- "$3"
  output_dir="$(realpath "$3")"
fi

[[ -d "$launcher_dir" ]] || fail "launcher package directory does not exist: $launcher_dir"
[[ -d "$search_dir" ]] || fail "search package directory does not exist: $search_dir"
[[ -d "$output_dir" ]] || fail "output directory does not exist: $output_dir"

for tool in dpkg dpkg-deb dpkg-query findmnt lsblk lscpu free sha256sum realpath python3 timeout; do
  command -v "$tool" >/dev/null 2>&1 || fail "required tool is unavailable: $tool"
done
[[ -f /etc/os-release ]] || fail 'Ubuntu 24.04 OS provenance is unavailable'
# shellcheck disable=SC1091
source /etc/os-release
[[ "${ID:-}" == ubuntu && "${VERSION_ID:-}" == 24.04 ]] || fail "requires Ubuntu 24.04, found ${ID:-unknown} ${VERSION_ID:-unknown}"

find_package() {
  local directory="$1" wanted="$2" candidate package matches=()
  shopt -s nullglob
  local candidates=("$directory"/*.deb)
  for candidate in "${candidates[@]}"; do
    package="$(dpkg-deb --field "$candidate" Package 2>/dev/null || true)"
    if [[ "$package" == "$wanted" ]]; then matches+=("$candidate"); fi
  done
  if [[ ${#matches[@]} -ne 1 ]]; then
    fail "expected exactly one $wanted Debian artifact in $directory, found ${#matches[@]}"
  fi
  printf '%s\n' "${matches[0]}"
}

launcher_deb="$(find_package "$launcher_dir" "$LAUNCHER_PACKAGE")"
search_deb="$(find_package "$search_dir" "$SEARCH_PACKAGE")"
host_arch="$(dpkg --print-architecture)"
for specification in "$LAUNCHER_PACKAGE|$LAUNCHER_BINARY|$launcher_deb" "$SEARCH_PACKAGE|$SEARCH_BINARY|$search_deb"; do
  IFS='|' read -r package_name binary package <<<"$specification"
  deb_name="$(dpkg-deb --field "$package" Package)"
  deb_version="$(dpkg-deb --field "$package" Version)"
  deb_arch="$(dpkg-deb --field "$package" Architecture)"
  [[ "$deb_name" == "$package_name" && -n "$deb_version" && "$deb_arch" == "$host_arch" ]] || \
    fail "unexpected artifact metadata for $package: package=$deb_name version=$deb_version architecture=$deb_arch host=$host_arch"
  installed="$(dpkg-query --show --showformat='${Status}|${Version}|${Architecture}' "$package_name" 2>/dev/null || true)"
  expected="install ok installed|$deb_version|$deb_arch"
  [[ "$installed" == "$expected" ]] || fail "installed $package_name does not match exact artifact $package: got ${installed:-not installed}, expected $expected"
  resolved="$(command -v "$(basename "$binary")" 2>/dev/null || true)"
  [[ "$resolved" == "$binary" && -x "$binary" && "$(realpath "$binary")" == "$binary" ]] || \
    fail "expected exact installed executable $binary, got ${resolved:-not found}"
  owner="$(dpkg-query --search "$binary" 2>/dev/null || true)"
  [[ "$owner" == "$package_name: $binary" ]] || fail "$binary is not owned solely by $package_name: ${owner:-no owner}"
done

for name in provenance.json hardware.json fixture.json fixture-manifest.json indexing-wait.json daemon-process.json observations.jsonl resource-samples.jsonl resource-summary.json summary.json native-ui-evidence.json desktop-catalog-validation.json native-cold-first-result.xwd launcher.log daemon.log; do
  [[ ! -e "$output_dir/$name" ]] || fail "refusing to overwrite existing benchmark output: $output_dir/$name"
done

if [[ "$private_session" != true ]]; then
  for tool in xvfb-run dbus-run-session jwm xdotool xclip xwd gdbus xprop; do
    command -v "$tool" >/dev/null 2>&1 || fail "required private native-session tool is unavailable: $tool"
  done
  command -v bwrap >/dev/null 2>&1 || fail 'Bubblewrap is required; refusing any unsandboxed fallback'
  command -v /usr/libexec/at-spi2-registryd >/dev/null 2>&1 || command -v /usr/lib/at-spi2-registryd >/dev/null 2>&1 || \
    fail 'at-spi2-registryd is required for native AT-SPI observation'
  [[ -f "$DRIVER" ]] || fail "benchmark driver is missing: $DRIVER"

  private_root="$(mktemp -d "$output_dir/.installed-native-benchmark.XXXXXX")"
  chmod 700 "$private_root"
  mkdir -m 700 "$private_root/tmp"
  cleanup_outer() {
    if [[ -s "$private_root/cleanup-failures" ]]; then
      printf 'preserving private session state after cleanup failure: %s\n' "$private_root" >&2
      return 0
    fi
    rm -rf -- "$private_root"
  }
  trap cleanup_outer EXIT
  script="$(realpath "${BASH_SOURCE[0]}")"
  if TMPDIR="$private_root/tmp" xvfb-run -a -s '-screen 0 1280x800x24 -nolisten tcp' \
    dbus-run-session -- bash "$script" --private-session "$private_root" "$launcher_dir" "$search_dir" "$output_dir"; then
    exit 0
  else
    status=$?
    exit "$status"
  fi
fi

[[ -d "$private_root/tmp" && "${DISPLAY:-}" =~ ^:[0-9]+$ ]] || fail 'internal private Xvfb session is not active'
[[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]] || fail 'internal private D-Bus session is not active'
export PATH=/usr/bin:/bin
export HOME="$private_root/home"
export XDG_CONFIG_HOME="$private_root/config"
export XDG_CACHE_HOME="$private_root/cache"
export XDG_DATA_HOME="$private_root/data"
export XDG_STATE_HOME="$private_root/state"
export XDG_RUNTIME_DIR="$private_root/runtime"
export XDG_DATA_DIRS="$private_root/data"
export XDG_CONFIG_DIRS="$private_root/config-system"
export TMPDIR="$private_root/tmp"
export XDG_CURRENT_DESKTOP=GNOME
export GDK_BACKEND=x11
export WINIT_UNIX_BACKEND=x11
export WAYLAND_DISPLAY=
export WAYLAND_SOCKET=
export AT_SPI_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"
export GIO_USE_PORTALS=0
export GTK_USE_PORTAL=0
export GSETTINGS_SCHEMA_DIR=/usr/share/glib-2.0/schemas
export LANG=C.UTF-8
export LC_ALL=C.UTF-8
export DBUS_SYSTEM_BUS_ADDRESS="unix:path=$private_root/no-system-bus"
unset DBUS_STARTER_ADDRESS DBUS_STARTER_BUS_TYPE SSH_AUTH_SOCK GPG_AGENT_INFO PULSE_SERVER SESSION_MANAGER || true
for directory in "$HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME" "$XDG_RUNTIME_DIR" "$private_root/config-system"; do
  mkdir -m 700 -p -- "$directory"
done
[[ -d "$GSETTINGS_SCHEMA_DIR" ]] || fail "system GSettings schemas are unavailable: $GSETTINGS_SCHEMA_DIR"
python3 - <<'PY'
import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi
PY

approved_root="$private_root/approved-root"
instance="$private_root/search-instance"
credential_file="$private_root/search-grant.credential"
socket_path="$instance/system/daemon.sock"
resource_targets_file="$private_root/resource-targets.jsonl"
mkdir -m 700 -p "$approved_root" "$private_root/data/applications" "$private_root/config/io.github.briolabs.Maestria.Launcher"
: >"$resource_targets_file"

umask 077
python3 "$DRIVER" prepare \
  --approved-root "$approved_root" \
  --data-home "$XDG_DATA_HOME" \
  --output-dir "$output_dir"
python3 "$DRIVER" provenance \
  --launcher-package "$LAUNCHER_PACKAGE" --launcher-deb "$launcher_deb" --launcher-binary "$LAUNCHER_BINARY" \
  --search-package "$SEARCH_PACKAGE" --search-deb "$search_deb" --search-binary "$SEARCH_BINARY" \
  --approved-root "$approved_root" --output-dir "$output_dir"

bubblewrap_args=(
  --unshare-user --unshare-pid --unshare-net --unshare-ipc --unshare-uts
  --disable-userns --cap-drop ALL --die-with-parent --new-session --clearenv
  --setenv HOME /home/benchmark --setenv LANG C.UTF-8 --setenv PATH /usr/bin:/bin
  --ro-bind /usr /usr --ro-bind-try /lib /lib --ro-bind-try /lib64 /lib64
  --proc /proc --dev /dev --dir /home --size 16777216 --tmpfs /tmp
)
printf '%s\n' 'private native benchmark host-only storage canary' >"$private_root/host-only-canary.txt"
if ! timeout --kill-after=2s 10s /usr/bin/bwrap "${bubblewrap_args[@]}" -- /usr/bin/true >"$private_root/bwrap-preflight.log" 2>&1; then
  cat "$private_root/bwrap-preflight.log" >&2
  fail 'Bubblewrap user/pid/network namespace preflight failed; refusing to run without the required isolation'
fi
if ! timeout --kill-after=2s 10s /usr/bin/bwrap "${bubblewrap_args[@]}" -- /usr/bin/test ! -e "$private_root/host-only-canary.txt" >"$private_root/bwrap-canary.log" 2>&1; then
  cat "$private_root/bwrap-canary.log" >&2
  fail 'Bubblewrap sandbox exposed the host-only filesystem canary'
fi

registry=/usr/libexec/at-spi2-registryd
[[ -x "$registry" ]] || registry=/usr/lib/at-spi2-registryd
registry_pid=
window_manager_pid=
daemon_pid=
monitor_pid=
process_is_live() {
  local pid="$1" stat_line state
  [[ -r "/proc/$pid/stat" ]] || return 1
  IFS= read -r stat_line <"/proc/$pid/stat" || return 1
  stat_line="${stat_line##*) }"
  state="${stat_line%% *}"
  [[ "$state" != Z && "$state" != X ]]
}

wait_for_private_process_exit() {
  local pid="$1" attempts="$2" attempt
  for ((attempt = 0; attempt < attempts; attempt++)); do
    if ! process_is_live "$pid"; then
      wait "$pid" 2>/dev/null || true
      return 0
    fi
    sleep 0.05
  done
  return 1
}

cleanup_processes() {
  local status=$?
  local pid attempt launcher_pid
  local -a cleanup_failures=()
  trap - EXIT INT TERM
  if [[ -s "$private_root/launcher.pid" ]]; then
    launcher_pid="$(<"$private_root/launcher.pid")"
    if [[ "$launcher_pid" =~ ^[0-9]+$ ]]; then
      kill -TERM -- "-$launcher_pid" 2>/dev/null || true
      for attempt in {1..100}; do
        kill -0 -- "-$launcher_pid" 2>/dev/null || break
        sleep 0.05
      done
      if kill -0 -- "-$launcher_pid" 2>/dev/null; then
        kill -KILL -- "-$launcher_pid" 2>/dev/null || true
        for attempt in {1..20}; do
          kill -0 -- "-$launcher_pid" 2>/dev/null || break
          sleep 0.05
        done
        if kill -0 -- "-$launcher_pid" 2>/dev/null; then
          cleanup_failures+=("launcher process group $launcher_pid survived SIGKILL")
        fi
      fi
    else
      cleanup_failures+=("invalid launcher process-group marker")
    fi
  fi
  if [[ -n "$monitor_pid" ]]; then
    kill -TERM "$monitor_pid" 2>/dev/null || true
    if ! wait_for_private_process_exit "$monitor_pid" 200; then
      kill -KILL "$monitor_pid" 2>/dev/null || true
      if ! wait_for_private_process_exit "$monitor_pid" 20; then
        cleanup_failures+=("resource monitor process $monitor_pid survived SIGKILL")
      fi
    fi
  fi
  if [[ -n "$daemon_pid" ]]; then
    kill -TERM -- "-$daemon_pid" 2>/dev/null || kill -TERM "$daemon_pid" 2>/dev/null || true
    if ! wait_for_private_process_exit "$daemon_pid" 100; then
      kill -KILL -- "-$daemon_pid" 2>/dev/null || kill -KILL "$daemon_pid" 2>/dev/null || true
      if ! wait_for_private_process_exit "$daemon_pid" 20; then
        cleanup_failures+=("search daemon process $daemon_pid survived SIGKILL")
      fi
    fi
    if kill -0 -- "-$daemon_pid" 2>/dev/null; then
      kill -KILL -- "-$daemon_pid" 2>/dev/null || true
      for attempt in {1..20}; do
        kill -0 -- "-$daemon_pid" 2>/dev/null || break
        sleep 0.05
      done
      if kill -0 -- "-$daemon_pid" 2>/dev/null; then
        cleanup_failures+=("search daemon process group $daemon_pid survived SIGKILL")
      fi
    fi
  fi
  for pid in "$window_manager_pid" "$registry_pid"; do
    if [[ -n "$pid" ]]; then
      kill -TERM "$pid" 2>/dev/null || true
      if ! wait_for_private_process_exit "$pid" 100; then
        kill -KILL "$pid" 2>/dev/null || true
        if ! wait_for_private_process_exit "$pid" 20; then
          cleanup_failures+=("private session process $pid survived SIGKILL")
        fi
      fi
    fi
  done
  if ((${#cleanup_failures[@]} > 0)); then
    printf '%s\n' "${cleanup_failures[@]}" >"$private_root/cleanup-failures"
  fi
  if [[ -n "$monitor_pid" ]]; then
    if ! python3 - "$output_dir/summary.json" "$output_dir/resource-summary.json" "$output_dir/resource-samples.jsonl" "$status" "${cleanup_failures[@]}" <<'PY'
import json, os, sys
from pathlib import Path

summary_path, resource_path, samples_path = map(Path, sys.argv[1:4])
session_exit_status = int(sys.argv[4])
cleanup_failures = sys.argv[5:]
valid = False
sample_count = 0
error_text = None
try:
    with resource_path.open(encoding="utf-8") as stream:
        resource = json.load(stream)
    with samples_path.open(encoding="utf-8") as stream:
        sample_count = sum(1 for line in stream if line.strip())
    valid = (
        resource.get("passed") is True
        and resource.get("sample_count") == sample_count
        and sample_count > 0
    )
    if not valid:
        error_text = "resource summary is failed or does not match the recorded samples"
except Exception as error:
    error_text = f"{type(error).__name__}: {error}"

if summary_path.is_file():
    with summary_path.open(encoding="utf-8") as stream:
        summary = json.load(stream)
    summary["resource_telemetry_gate"] = {
        "passed": valid,
        "sample_count": sample_count,
        "summary_file": str(resource_path),
        "samples_file": str(samples_path),
        "error": error_text,
    }
    summary["private_cleanup_gate"] = {
        "passed": not cleanup_failures,
        "failures": cleanup_failures,
    }
    summary["private_session_exit_gate"] = {
        "passed": session_exit_status == 0,
        "exit_status": session_exit_status,
    }
    if not valid or cleanup_failures or session_exit_status != 0:
        summary["result"] = "FAIL"
        failures = summary.setdefault("acceptance_failures", [])
        if not valid:
            failures.append(f"resource telemetry gate failed: {error_text or 'unknown resource sampler error'}")
        failures.extend(f"private cleanup gate failed: {failure}" for failure in cleanup_failures)
        if session_exit_status != 0:
            failures.append(f"private session exited with status {session_exit_status}")
    temporary_path = summary_path.with_name(f".{summary_path.name}.tmp")
    with temporary_path.open("w", encoding="utf-8") as stream:
        json.dump(summary, stream, ensure_ascii=False, indent=2, sort_keys=True)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary_path, summary_path)

if not valid or cleanup_failures or session_exit_status != 0:
    if not valid:
        print(f"resource telemetry validation failed: {error_text or 'unknown error'}", file=sys.stderr)
    for failure in cleanup_failures:
        print(f"private cleanup validation failed: {failure}", file=sys.stderr)
    if session_exit_status != 0:
        print(f"private session exited with status {session_exit_status}", file=sys.stderr)
    raise SystemExit(1)
PY
    then
      status=1
    fi
  fi
  exit "$status"
}
trap cleanup_processes EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
python3 "$DRIVER" monitor --output-dir "$output_dir" --resource-targets-file "$resource_targets_file" --interval-ms 100 &
monitor_pid=$!
for attempt in {1..20}; do
  [[ -s "$output_dir/resource-samples.jsonl" ]] && break
  kill -0 "$monitor_pid" 2>/dev/null || fail 'resource telemetry sampler exited before collecting data'
  sleep 0.05
done
[[ -s "$output_dir/resource-samples.jsonl" ]] || fail 'resource telemetry sampler did not start'

printf '%s\n' '<JWM><FocusModel>click</FocusModel></JWM>' >"$private_root/jwm.xml"
jwm -f "$private_root/jwm.xml" >"$private_root/jwm.log" 2>&1 &
window_manager_pid=$!
wm_ready=false
for attempt in {1..100}; do
  if ! kill -0 "$window_manager_pid" 2>/dev/null; then
    cat "$private_root/jwm.log" >&2
    fail 'private JWM exited before becoming ready'
  fi
  if [[ "$(xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null || true)" == *'window id # 0x'* ]]; then
    wm_ready=true
    break
  fi
  sleep 0.05
done
[[ "$wm_ready" == true ]] || fail 'private X11 window manager did not become ready'

"$registry" >"$private_root/at-spi-registry.log" 2>&1 &
registry_pid=$!
registry_ready=false
for attempt in {1..100}; do
  owner="$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner org.a11y.atspi.Registry 2>/dev/null || true)"
  if [[ "$owner" == *true* ]]; then registry_ready=true; break; fi
  sleep 0.05
done
if [[ "$registry_ready" != true ]]; then
  cat "$private_root/at-spi-registry.log" >&2
  fail 'private AT-SPI registry did not become ready'
fi
gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
  --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled '<true>' >/dev/null
portal_owner="$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner org.freedesktop.portal.Desktop 2>/dev/null || true)"
[[ "$portal_owner" != *true* ]] || fail 'optional desktop portal unexpectedly owns a name on the private session bus'

"$SEARCH_BINARY" init --instance-dir "$instance" --read-root "$approved_root" >"$private_root/search-init.out" 2>"$private_root/search-init.err" || {
  cat "$private_root/search-init.err" >&2
  fail 'installed search binary could not initialize the exact private approved root'
}
provider_realm="$(sed -n 's/^realm_id=//p' "$instance/manifest.txt")"
[[ "$provider_realm" =~ ^[0-9a-f]{64}$ ]] || fail 'private search instance has no valid provider realm'
consumer_realm=
for candidate in {1..64}; do
  candidate_realm="$(printf '%064x' "$candidate")"
  if [[ "$candidate_realm" != "$provider_realm" ]]; then
    consumer_realm="$candidate_realm"
    break
  fi
done
[[ -n "$consumer_realm" ]] || fail 'could not choose a consumer realm distinct from the provider'

setsid "$SEARCH_BINARY" start --instance-dir "$instance" >"$output_dir/daemon.log" 2>&1 &
daemon_pid=$!
python3 "$DRIVER" record-process \
  --resource-targets-file "$resource_targets_file" --pid "$daemon_pid" \
  --expected-binary "$SEARCH_BINARY"
owner_ready=false
for attempt in {1..600}; do
  if "$SEARCH_BINARY" owner roots status --instance-dir "$instance" >"$private_root/owner-roots.json" 2>"$private_root/owner-roots.err"; then
    if python3 - "$private_root/owner-roots.json" "$approved_root" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as stream:
    status = json.load(stream)
roots = status.get("roots")
if status.get("approved_root_count") == 1 and isinstance(roots, list) and len(roots) == 1 and roots[0].get("path") == sys.argv[2]:
    raise SystemExit(0)
raise SystemExit(1)
PY
    then owner_ready=true; break; fi
  fi
  if ! kill -0 "$daemon_pid" 2>/dev/null; then
    cat "$output_dir/daemon.log" >&2
    fail 'installed search daemon exited before exposing the single approved root'
  fi
  sleep 0.05
done
if [[ "$owner_ready" != true ]]; then
  cat "$output_dir/daemon.log" >&2
  cat "$private_root/owner-roots.err" >&2 || true
  fail 'installed search daemon did not expose exactly the approved read root'
fi
[[ -S "$socket_path" ]] || fail "installed search daemon did not bind its private socket: $socket_path"

if ! grant_output="$("$SEARCH_BINARY" owner grant create-external \
  --instance-dir "$instance" --consumer-realm "$consumer_realm" \
  --credential-file "$credential_file" --access search-and-open-evidence \
  --max-sensitivity internal --max-results 1 --max-evidence-bytes 512 \
  --expires-in-seconds 86400 --read-root "$approved_root" 2>"$private_root/grant-create.err")"; then
  cat "$private_root/grant-create.err" >&2
  fail 'installed search daemon could not create the bounded private external grant'
fi
for expected in "consumer_realm=$consumer_realm" 'access=search-and-open-evidence' 'max_results=1' 'max_evidence_bytes=512' 'state=active' "allowed_roots=[\"$approved_root\"]"; do
  [[ "$grant_output" == *"$expected"* ]] || fail "private search grant omitted exact scope field: $expected"
done
[[ -s "$credential_file" && "$(stat -c '%a' "$credential_file")" == 600 ]] || fail 'external grant credential is not private mode 0600'

cat >"$private_root/config/io.github.briolabs.Maestria.Launcher/launcher.toml" <<TOML
schemaVersion = 1
shortcut = "Control+Space"
shortcutSetup = "deferred"
reduceMotion = true
theme = "dark"

[search]
socketPath = "$socket_path"
consumerRealm = "$consumer_realm"
credentialFile = "$credential_file"
TOML

python3 "$DRIVER" wait-index \
  --search-binary "$SEARCH_BINARY" --socket-path "$socket_path" \
  --consumer-realm "$consumer_realm" --credential-file "$credential_file" \
  --expected-files 10000 --timeout-seconds 900 --output-dir "$output_dir"
python3 "$DRIVER" drive \
  --manifest "$output_dir/fixture.json" --output-dir "$output_dir" \
  --launcher-binary "$LAUNCHER_BINARY" --search-binary "$SEARCH_BINARY" \
  --socket-path "$socket_path" --consumer-realm "$consumer_realm" \
  --credential-file "$credential_file" --daemon-pid "$daemon_pid" \
  --process-state "$private_root/launcher.pid" \
  --resource-targets-file "$resource_targets_file" --sample-timeout-ms 5000

portal_owner="$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner org.freedesktop.portal.Desktop 2>/dev/null || true)"
[[ "$portal_owner" != *true* ]] || fail 'benchmark unexpectedly activated an optional desktop portal'
printf 'Installed %s %s and %s %s were measured through the private Slint X11 window and AT-SPI; observations and resource/hardware provenance are in %s\n' \
  "$LAUNCHER_PACKAGE" "$(dpkg-deb --field "$launcher_deb" Version)" \
  "$SEARCH_PACKAGE" "$(dpkg-deb --field "$search_deb" Version)" "$output_dir"

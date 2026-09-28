#!/usr/bin/env bash
set -euo pipefail

fail() { echo "combined extension package smoke failed: $*" >&2; exit 1; }

if [[ $# -eq 2 ]]; then
  launcher_dir="$(realpath "$1")"
  worker_dir="$(realpath "$2")"
  for tool in dpkg-deb dpkg-query bwrap xvfb-run dbus-run-session jwm xdotool xclip python3; do
    command -v "$tool" >/dev/null || fail "required tool is unavailable: $tool"
  done
  shopt -s nullglob
  launcher_packages=("$launcher_dir"/*.deb)
  worker_packages=("$worker_dir"/*.deb)
  [[ ${#launcher_packages[@]} -eq 1 && ${#worker_packages[@]} -eq 1 ]] || fail 'expected exactly one Debian package per component'
  for specification in \
    "io-github-briolabs-maestria-launcher|/usr/bin/maestria-launcher|${launcher_packages[0]}" \
    "io-github-briolabs-maestria-extension-worker|/usr/bin/maestria-extension-worker|${worker_packages[0]}"; do
    IFS='|' read -r package_name binary package <<<"$specification"
    [[ "$(dpkg-deb --field "$package" Package)" == "$package_name" ]] || fail "incorrect package name: $package"
    installed="$(dpkg-query --show --showformat='${Status}|${Version}|${Architecture}' "$package_name" 2>/dev/null || true)"
    expected="install ok installed|$(dpkg-deb --field "$package" Version)|$(dpkg-deb --field "$package" Architecture)"
    [[ "$installed" == "$expected" ]] || fail "installed $package_name does not match exact artifact $package: $installed"
    [[ -x "$binary" && "$(dpkg-query --search "$binary")" == "$package_name: $binary" ]] || fail "package-owned executable missing: $binary"
  done
  [[ ! -e /usr/bin/maestria-search ]] || fail 'combined launcher+worker smoke unexpectedly installed optional search'
  root="$(mktemp -d -t sillage-combined-extension.XXXXXX)"
  cleanup_root() { chmod -R u+w -- "$root" 2>/dev/null || true; rm -rf -- "$root"; }
  trap cleanup_root EXIT
  mkdir -m 700 "$root"/{home,config,cache,data,state,runtime}
  export HOME="$root/home" XDG_CONFIG_HOME="$root/config" XDG_CACHE_HOME="$root/cache"
  export XDG_DATA_HOME="$root/data" XDG_STATE_HOME="$root/state" XDG_RUNTIME_DIR="$root/runtime"
  export XDG_CURRENT_DESKTOP=GNOME
  unset WAYLAND_DISPLAY
  xvfb-run -a -s '-screen 0 1280x800x24' dbus-run-session -- bash "$0" "$launcher_dir" "$worker_dir" "$root"
  exit
fi
[[ $# -eq 3 ]] || fail 'usage: smoke-combined-extension-packages.sh LAUNCHER_PACKAGE_DIRECTORY WORKER_PACKAGE_DIRECTORY'
root="$3"
[[ -d "$root/home" && -d "$root/runtime" && "$XDG_RUNTIME_DIR" == "$root/runtime" ]] || fail 'missing private XDG runtime'
export PATH=/usr/bin:/bin GDK_BACKEND=x11 WINIT_UNIX_BACKEND=x11
export AT_SPI_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:?private D-Bus required}"
launcher_pid= wm_pid= registry_pid=
cleanup_processes() {
  if [[ -s "$root/restarted.pid" ]]; then
    kill -TERM -- "-$(cat "$root/restarted.pid")" 2>/dev/null || true
  fi
  for pid in "$launcher_pid" "$wm_pid" "$registry_pid"; do
    if [[ -n "$pid" ]]; then kill -TERM "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
  if [[ -s "$root/launcher.log" ]]; then cat "$root/launcher.log"; fi
  if [[ -s "$root/restarted.log" ]]; then cat "$root/restarted.log"; fi
}
trap cleanup_processes EXIT
mkdir -p "$root/config/io.github.briolabs.Maestria.Launcher" "$root/package/dist"
cat > "$root/config/io.github.briolabs.Maestria.Launcher/launcher.toml" <<'TOML'
schemaVersion = 1
shortcut = "Control+Space"
shortcutSetup = "deferred"
reduceMotion = true
TOML
cat > "$root/package/manifest.json" <<'JSON'
{"apiVersion":1,"id":"dev.sillage.private-broker","name":"Private Broker","version":"1.0.0","entrypoints":[{"id":"main","file":"dist/main.js"}],"commands":[{"id":"greetings","title":"Greetings","entrypointId":"main"}],"permissions":[{"type":"copy","formats":["text"]}]}
JSON
cat > "$root/package/dist/main.js" <<'JS'
export default {
  commands: {
    greetings: async ({ invocation, requestCapability }) => {
      if (invocation.kind === "action") {
        const result = await requestCapability({ capability: "copy", text: "private Sillage broker token" });
        return result.ok
          ? { kind: "detail", title: "Broker authorized copy", blocks: [{ kind: "text", text: "private Sillage broker token" }] }
          : { kind: "error", title: "Broker refused copy", message: result.error.message };
      }
      const denied = await requestCapability({ capability: "fileSearch", query: "orchid lantern", limit: 1 });
      if (denied.ok || denied.error.code !== "permission_denied") {
        return { kind: "error", title: "Unexpected file access", message: "Ungrantable file search was not denied." };
      }
      return { kind: "list", title: "Private Broker denied ungranted file search", items: [
        { id: "private", title: "Private greeting", actions: [{ id: "copy", label: "Copy private greeting", role: "primary" }] }
      ] };
    }
  }
};
JS
registry=/usr/libexec/at-spi2-registryd
[[ -x "$registry" ]] || registry=/usr/lib/at-spi2-registryd
[[ -x "$registry" ]] || fail 'AT-SPI registry unavailable'
"$registry" >"$root/registry.log" 2>&1 & registry_pid=$!
ready=false
for attempt in {1..100}; do
  owner="$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner org.a11y.atspi.Registry 2>/dev/null || true)"
  if [[ "$owner" == *true* ]]; then ready=true; break; fi
  sleep .05
done
[[ "$ready" == true ]] || fail 'private AT-SPI registry did not become ready'
gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
  --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled '<true>' >/dev/null
printf '%s\n' '<JWM><FocusModel>click</FocusModel></JWM>' >"$root/jwm.xml"
jwm -f "$root/jwm.xml" >"$root/jwm.log" 2>&1 & wm_pid=$!
ready=false
for attempt in {1..100}; do
  wm_check="$(xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null || true)"
  if [[ "$wm_check" == *'window id # 0x'* ]]; then ready=true; break; fi
  sleep .05
done
[[ "$ready" == true ]] || fail 'private X11 window manager did not become ready'
setsid /usr/bin/maestria-launcher --activate >"$root/launcher.log" 2>&1 & launcher_pid=$!
window=
for attempt in {1..100}; do
  window="$(xdotool search --onlyvisible --name '^Sillage Launcher$' 2>/dev/null | { read -r id; printf '%s' "$id"; } || true)"
  if [[ -n "$window" ]]; then break; fi
  if ! kill -0 "$launcher_pid" 2>/dev/null; then fail 'installed launcher exited before showing Slint'; fi
  sleep .05
done
[[ -n "$window" ]] || fail 'installed launcher did not show a private Slint window'
xdotool windowactivate --sync "$window"
python3 scripts/check-combined-extension-broker.py "$root" "$window"

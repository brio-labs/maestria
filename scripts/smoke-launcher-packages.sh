#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 || ! -d "$1" ]]; then
  echo "usage: smoke-launcher-packages.sh PACKAGE_DIRECTORY" >&2
  exit 2
fi

package_dir="$(realpath "$1")"
shopt -s nullglob
appimages=("$package_dir"/*.AppImage)
if [[ ${#appimages[@]} -ne 1 ]]; then
  echo "expected one AppImage in $package_dir, found ${#appimages[@]}" >&2
  exit 1
fi

LAUNCHER_PACKAGE=io-github-briolabs-maestria-launcher
SEARCH_PACKAGE=io-github-briolabs-maestria-search
WORKER_PACKAGE=io-github-briolabs-maestria-extension-worker
debs=("$package_dir"/*.deb)
if [[ ${#debs[@]} -ne 1 ]]; then
  echo "expected one Debian package in $package_dir, found ${#debs[@]}" >&2
  exit 1
fi
package="${debs[0]}"
package_name="$(dpkg-deb --field "$package" Package)"
package_version="$(dpkg-deb --field "$package" Version)"
package_architecture="$(dpkg-deb --field "$package" Architecture)"
if [[ "$package_name" != "$LAUNCHER_PACKAGE" || -z "$package_version" ]]; then
  echo "unexpected launcher Debian package metadata: package=$package_name version=$package_version" >&2
  exit 1
fi
installed="$(dpkg-query --show --showformat='${Status}|${Version}|${Architecture}' "$LAUNCHER_PACKAGE" 2>/dev/null || true)"
expected_installed="install ok installed|$package_version|$package_architecture"
if [[ "$installed" != "$expected_installed" ]]; then
  echo "installed launcher package does not match $package: got $installed" >&2
  exit 1
fi
for component_package in "$SEARCH_PACKAGE" "$WORKER_PACKAGE"; do
  component_status="$(dpkg-query --show --showformat='${Status}' "$component_package" 2>/dev/null || true)"
  if [[ "$component_status" == "install ok installed" ]]; then
    echo "launcher-only smoke requires $component_package to remain uninstalled" >&2
    exit 1
  fi
done
if [[ -x /usr/bin/maestria-search || -x /usr/bin/maestria-extension-worker ]]; then
  echo "launcher-only smoke requires search and extension worker binaries to be absent" >&2
  exit 1
fi
export PATH=/usr/bin:/bin

root="$(mktemp -d -t maestria-launcher-native-smoke.XXXXXX)"
mkdir -p "$root/home" "$root/config" "$root/cache" "$root/data" "$root/state" "$root/runtime"
chmod 700 "$root/runtime"
export HOME="$root/home"
export XDG_CONFIG_HOME="$root/config"
export XDG_CACHE_HOME="$root/cache"
export XDG_DATA_HOME="$root/data"
export XDG_STATE_HOME="$root/state"
export XDG_RUNTIME_DIR="$root/runtime"
export XDG_DATA_DIRS=/usr/share
export GDK_BACKEND=x11
export WINIT_UNIX_BACKEND=x11
export WAYLAND_DISPLAY=
export AT_SPI_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:?run this script inside dbus-run-session}"
export GSETTINGS_SCHEMA_DIR="${GSETTINGS_SCHEMA_DIR:-/usr/share/glib-2.0/schemas}"
mkdir -p "$root/data/applications"
cat > "$root/data/applications/sillage-package-smoke.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Sillage Package Smoke Application
Exec=/usr/bin/touch $root/app-launched
Terminal=false
EOF

window_manager_pid=
registry_pid=
active_app_pid=
grab_pid=
weston_pid=
cleanup() {
  if [[ -n "$active_app_pid" ]]; then
    kill -TERM -- "-$active_app_pid" 2>/dev/null || true
    sleep 0.2
    kill -KILL -- "-$active_app_pid" 2>/dev/null || true
    wait "$active_app_pid" 2>/dev/null || true
  fi
  if [[ -n "$grab_pid" ]]; then
    kill -TERM "$grab_pid" 2>/dev/null || true
    wait "$grab_pid" 2>/dev/null || true
  fi
  if [[ -n "$weston_pid" ]]; then
    kill -TERM "$weston_pid" 2>/dev/null || true
    wait "$weston_pid" 2>/dev/null || true
  fi
  if [[ -n "$window_manager_pid" ]]; then
    kill -TERM "$window_manager_pid" 2>/dev/null || true
    wait "$window_manager_pid" 2>/dev/null || true
  fi
  if [[ -n "$registry_pid" ]]; then
    kill -TERM "$registry_pid" 2>/dev/null || true
    wait "$registry_pid" 2>/dev/null || true
  fi
  rm -rf "$root"
}
trap cleanup EXIT

cat > "$root/jwm.xml" <<'EOF'
<JWM><FocusModel>click</FocusModel></JWM>
EOF
jwm -f "$root/jwm.xml" >"$root/jwm.log" 2>&1 &
window_manager_pid=$!
window_manager_ready=false
for attempt in {1..100}; do
  if ! kill -0 "$window_manager_pid" 2>/dev/null; then
    cat "$root/jwm.log" >&2
    echo "X11 window manager exited before becoming ready" >&2
    exit 1
  fi
  wm_check="$(xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null || true)"
  if [[ "$wm_check" == *"window id # 0x"* ]]; then
    window_manager_ready=true
    break
  fi
  sleep 0.05
done
if [[ "$window_manager_ready" != true ]]; then
  cat "$root/jwm.log" >&2
  echo "X11 window manager did not become ready" >&2
  exit 1
fi

registry=/usr/libexec/at-spi2-registryd
if [[ ! -x "$registry" ]]; then
  registry=/usr/lib/at-spi2-registryd
fi
if [[ ! -x "$registry" ]]; then
  echo "at-spi2-registryd is not installed" >&2
  exit 1
fi
"$registry" >"$root/at-spi-registry.log" 2>&1 &
registry_pid=$!

registry_ready=false
for attempt in {1..100}; do
  if ! kill -0 "$registry_pid" 2>/dev/null; then
    cat "$root/at-spi-registry.log" >&2
    echo "AT-SPI registry exited before becoming ready" >&2
    exit 1
  fi
  owner="$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner org.a11y.atspi.Registry 2>/dev/null || true)"
  if [[ "$owner" == *true* ]]; then
    registry_ready=true
    break
  fi
  sleep 0.05
done
if [[ "$registry_ready" != true ]]; then
  cat "$root/at-spi-registry.log" >&2
  echo "AT-SPI registry did not become ready" >&2
  exit 1
fi

# AccessKit only publishes Slint's tree after the private session enables AT-SPI.
gdbus call --session \
  --dest org.a11y.Bus \
  --object-path /org/a11y/bus \
  --method org.freedesktop.DBus.Properties.Set \
  org.a11y.Status IsEnabled '<true>'

smoke_window() {
  local label="$1"
  shift
  local log="$root/$label.log"
  local window=
  local query="native packaging smoke"

  setsid "$@" --activate >"$log" 2>&1 &
  active_app_pid=$!
  for attempt in {1..120}; do
    if ! kill -0 "$active_app_pid" 2>/dev/null; then
      break
    fi
    window="$(xdotool search --onlyvisible --name '^Sillage Launcher$' 2>/dev/null | { read -r id; printf '%s' "$id"; } || true)"
    if [[ -n "$window" ]]; then
      break
    fi
    sleep 0.25
  done
  if [[ -z "$window" ]]; then
    cat "$log" >&2
    echo "$label package did not show a visible Sillage Launcher window" >&2
    return 1
  fi

  xdotool windowactivate --sync "$window"
  xdotool windowfocus --sync "$window"
  if [[ "$label" == deb ]]; then
    if ! timeout 20s python3 scripts/check-launcher-accessibility.py --offer; then
      cat "$log" >&2
      return 1
    fi
  else
    timeout 20s python3 scripts/check-launcher-accessibility.py --no-offer
  fi
  xdotool type --window "$window" --clearmodifiers "$query"
  timeout 20s python3 scripts/check-launcher-accessibility.py "$query"
  if [[ "$label" == deb ]]; then
    timeout 20s python3 scripts/check-launcher-accessibility.py --defer-offer
  fi
  xdotool windowactivate --sync "$window"
  xdotool windowfocus --sync "$window"
  xdotool key --clearmodifiers ctrl+a
  xdotool type --clearmodifiers "2 + 2"
  timeout 20s python3 scripts/check-launcher-accessibility.py --calculation
  xdotool key --clearmodifiers Return
  xdotool key --clearmodifiers ctrl+a
  xdotool key --clearmodifiers ctrl+v
  timeout 20s python3 scripts/check-launcher-accessibility.py "4"
  xdotool key --clearmodifiers ctrl+comma
  if ! timeout 20s python3 scripts/check-launcher-accessibility.py --preferences; then
    cat "$log" >&2
    return 1
  fi
  timeout 20s python3 scripts/check-launcher-accessibility.py --reset-cancel
  timeout 20s python3 scripts/check-launcher-accessibility.py --reset-begin
  "$@" --activate
  timeout 20s python3 scripts/check-launcher-accessibility.py --reset-hidden
  xdotool key --clearmodifiers ctrl+comma
  timeout 20s python3 scripts/check-launcher-accessibility.py --reset-cleared

  xdotool key --clearmodifiers Escape
  xdotool key --clearmodifiers Escape
  for attempt in {1..50}; do
    if ! xdotool search --onlyvisible --name '^Sillage Launcher$' >/dev/null 2>&1; then
      break
    fi
    sleep 0.1
  done
  if xdotool search --onlyvisible --name '^Sillage Launcher$' >/dev/null 2>&1; then
    echo "$label did not dismiss its native window" >&2
    return 1
  fi
  local resident_pid
  resident_pid="$(xdotool getwindowpid "$window")"
  "$@" --activate
  local reopened=
  for attempt in {1..50}; do
    reopened="$(xdotool search --onlyvisible --name '^Sillage Launcher$' 2>/dev/null | { read -r id; printf '%s' "$id"; } || true)"
    if [[ -n "$reopened" ]]; then
      break
    fi
    sleep 0.1
  done
  if [[ "$reopened" != "$window" || "$(xdotool getwindowpid "$reopened")" != "$resident_pid" ]]; then
    echo "$label --activate did not reuse the resident window and process" >&2
    return 1
  fi
  timeout 20s python3 scripts/check-launcher-accessibility.py ""
  "$@" --quit
  for attempt in {1..50}; do
    if ! kill -0 "$active_app_pid" 2>/dev/null; then
      break
    fi
    sleep 0.1
  done
  if kill -0 "$active_app_pid" 2>/dev/null; then
    echo "$label --quit did not stop its resident process" >&2
    return 1
  fi

  kill -TERM -- "-$active_app_pid" 2>/dev/null || true
  wait "$active_app_pid" 2>/dev/null || true
  active_app_pid=
  echo "$label package native keyboard, calculation/clipboard, first-run deferral, Preferences, resident reactivation/quit, and AT-SPI smoke passed"
}

smoke_window deb /usr/bin/maestria-launcher
smoke_window appimage env APPIMAGE_EXTRACT_AND_RUN=1 "${appimages[0]}"

wait_launcher_window() {
  local window=
  for attempt in {1..80}; do
    window="$(xdotool search --onlyvisible --name '^Sillage Launcher$' 2>/dev/null | { read -r id; printf '%s' "$id"; } || true)"
    if [[ -n "$window" ]]; then
      printf '%s\n' "$window"
      return 0
    fi
    sleep 0.1
  done
  echo "native shortcut did not show the launcher window" >&2
  return 1
}

wait_launcher_hidden() {
  for attempt in {1..80}; do
    if ! xdotool search --onlyvisible --name '^Sillage Launcher$' >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  echo "native launcher window did not hide before shortcut activation" >&2
  return 1
}

start_conflicting_grab() {
  python3 -u - "$root/grab.ready" >"$root/grab.log" 2>&1 <<'PY' &
import ctypes
from pathlib import Path
import sys
import time

x11 = ctypes.CDLL("libX11.so.6")
x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
x11.XOpenDisplay.restype = ctypes.c_void_p
x11.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x11.XDefaultRootWindow.restype = ctypes.c_ulong
x11.XStringToKeysym.argtypes = [ctypes.c_char_p]
x11.XStringToKeysym.restype = ctypes.c_ulong
x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x11.XKeysymToKeycode.restype = ctypes.c_uint
x11.XGrabKey.argtypes = [
    ctypes.c_void_p, ctypes.c_int, ctypes.c_uint, ctypes.c_ulong,
    ctypes.c_int, ctypes.c_int, ctypes.c_int,
]
x11.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
x11.XSetErrorHandler.argtypes = [ctypes.c_void_p]
errors = []
handler_type = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p)


@handler_type
def on_x11_error(_display, _event):
    errors.append(True)
    return 0


x11.XSetErrorHandler(on_x11_error)
display = x11.XOpenDisplay(None)
if not display:
    raise SystemExit("could not open isolated X11 display for conflicting shortcut")
keycode = x11.XKeysymToKeycode(display, x11.XStringToKeysym(b"F12"))
if not keycode:
    raise SystemExit("X11 display has no F12 keycode")
x11.XGrabKey(display, keycode, 4 | 8, x11.XDefaultRootWindow(display), 0, 1, 1)
x11.XSync(display, 0)
if errors:
    raise SystemExit("could not reserve Control+Alt+F12 for conflicting shortcut")
Path(sys.argv[1]).touch()
time.sleep(60)
PY
  grab_pid=$!
  for attempt in {1..100}; do
    [[ -f "$root/grab.ready" ]] && return 0
    if ! kill -0 "$grab_pid" 2>/dev/null; then
      cat "$root/grab.log" >&2
      return 1
    fi
    sleep 0.05
  done
  cat "$root/grab.log" >&2
  return 1
}

smoke_shortcut_setup() {
  export XDG_CONFIG_HOME="$root/shortcut-config"
  mkdir -p "$XDG_CONFIG_HOME"
  setsid maestria-launcher --activate >"$root/shortcut-setup.log" 2>&1 &
  active_app_pid=$!
  local window
  window="$(wait_launcher_window)"
  xdotool windowactivate --sync "$window"
  xdotool windowfocus --sync "$window"
  timeout 20s python3 scripts/check-launcher-accessibility.py --offer
  timeout 20s python3 scripts/check-launcher-accessibility.py --setup-offer

  xdotool key --clearmodifiers Escape
  wait_launcher_hidden
  for lock_key in Caps_Lock Num_Lock; do
    xdotool key --clearmodifiers "$lock_key"
    xdotool key --clearmodifiers ctrl+space
    if [[ "$(wait_launcher_window)" != "$window" ]]; then
      echo "$lock_key changed the resident shortcut's target window" >&2
      return 1
    fi
    xdotool key --clearmodifiers "$lock_key"
    xdotool key --clearmodifiers Escape
    wait_launcher_hidden
  done

  maestria-launcher --quit
  wait "$active_app_pid" 2>/dev/null || true
  active_app_pid=
  setsid maestria-launcher --activate >"$root/shortcut-restart.log" 2>&1 &
  active_app_pid=$!
  window="$(wait_launcher_window)"
  timeout 20s python3 scripts/check-launcher-accessibility.py --requested-shortcut
  xdotool key --clearmodifiers Escape
  wait_launcher_hidden
  xdotool key --clearmodifiers ctrl+space
  if [[ "$(wait_launcher_window)" != "$window" ]]; then
    echo "saved shortcut was not restored after native launcher restart" >&2
    return 1
  fi
  start_conflicting_grab
  xdotool key --clearmodifiers ctrl+comma
  timeout 20s python3 scripts/check-launcher-accessibility.py --preferences
  xdotool key --clearmodifiers ctrl+a
  xdotool type --clearmodifiers "Control+Alt+F12"
  timeout 20s python3 scripts/check-launcher-accessibility.py --save-preferences
  timeout 20s python3 scripts/check-launcher-accessibility.py --conflict-rejected
  kill -TERM "$grab_pid"
  wait "$grab_pid" 2>/dev/null || true
  grab_pid=
  xdotool key --clearmodifiers Escape
  xdotool key --clearmodifiers Escape
  wait_launcher_hidden
  xdotool key --clearmodifiers ctrl+space
  if [[ "$(wait_launcher_window)" != "$window" ]]; then
    echo "failed conflicting shortcut changed the active saved shortcut" >&2
    return 1
  fi

  xdotool key --clearmodifiers ctrl+a
  xdotool type --clearmodifiers "Sillage Package Smoke Application"
  timeout 20s python3 scripts/check-launcher-accessibility.py --application
  xdotool key --clearmodifiers Return
  local launched=false
  for attempt in {1..80}; do
    if [[ -f "$root/app-launched" ]]; then
      launched=true
      break
    fi
    sleep 0.1
  done
  if [[ "$launched" != true ]]; then
    echo "native launcher did not launch the approved XDG desktop-entry fixture" >&2
    return 1
  fi
  wait_launcher_hidden
  maestria-launcher --quit
  wait "$active_app_pid" 2>/dev/null || true
  active_app_pid=
  echo "native X11 shortcut persisted, survived restart and lock modifiers, and rejected conflicting grabs"
}

smoke_shortcut_setup

smoke_nested_wayland() {
  mkdir -p "$root/wayland-config"
  export XDG_CONFIG_HOME="$root/wayland-config"
  export WAYLAND_DISPLAY=sillage-smoke-wayland
  export DISPLAY=
  export GDK_BACKEND=wayland
  export WINIT_UNIX_BACKEND=wayland
  local fake_seat=false
  local -a weston_args=(
    --backend=headless --renderer=pixman --no-config
    --socket="$WAYLAND_DISPLAY" --width=1280 --height=800 --idle-time=0
  )
  if [[ "${SILLAGE_TEST_WESTON_NO_FAKE_SEAT:-0}" != 1 ]] &&
    [[ "$(weston --help)" == *"--fake-seat"* ]]; then
    weston_args+=(--fake-seat)
    fake_seat=true
  fi
  weston "${weston_args[@]}" >"$root/weston.log" 2>&1 &
  weston_pid=$!
  local socket_ready=false
  for attempt in {1..100}; do
    if [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]; then
      socket_ready=true
      break
    fi
    if ! kill -0 "$weston_pid" 2>/dev/null; then
      cat "$root/weston.log" >&2
      echo "isolated Weston Wayland compositor failed to start" >&2
      return 1
    fi
    sleep 0.05
  done
  if [[ "$socket_ready" != true ]]; then
    cat "$root/weston.log" >&2
    echo "isolated Weston Wayland socket was not ready" >&2
    return 1
  fi

  setsid maestria-launcher --activate >"$root/wayland-launcher.log" 2>&1 &
  active_app_pid=$!
  timeout 20s python3 scripts/check-launcher-accessibility.py --offer
  if [[ "$fake_seat" == true ]]; then
    timeout 20s python3 scripts/check-launcher-accessibility.py --defer-offer
  else
    timeout 20s python3 scripts/check-launcher-accessibility.py --defer-offer-no-seat
  fi
  maestria-launcher --activate
  timeout 20s python3 scripts/check-launcher-accessibility.py --no-offer
  maestria-launcher --quit
  wait "$active_app_pid"
  active_app_pid=
  kill -TERM "$weston_pid"
  wait "$weston_pid" 2>/dev/null || true
  weston_pid=
  echo "nested Weston Wayland package startup, AT-SPI offer/deferral, resident reactivation and quit passed (fake-seat=$fake_seat)"
}
smoke_nested_wayland

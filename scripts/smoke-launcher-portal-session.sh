#!/usr/bin/env bash
set -euo pipefail

[[ $# -eq 2 && ( "$1" == allow || "$1" == deny ) && -d "$2/runtime" ]] || {
  echo 'usage: smoke-launcher-portal-session.sh allow|deny PRIVATE_SESSION_ROOT' >&2
  exit 2
}
decision="$1" session="$2"
[[ "$XDG_RUNTIME_DIR" == "$session/runtime" && -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]] || {
  echo 'private runtime and D-Bus are required' >&2; exit 1;
}
outer_display="${DISPLAY:?private Xvfb display required}"
export WAYLAND_DISPLAY=sillage-private-portal-wayland
export GDK_BACKEND=wayland WINIT_UNIX_BACKEND=wayland QT_QPA_PLATFORM=wayland
export AT_SPI_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"
export QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1
export KDE_FULL_SESSION=true KDE_SESSION_VERSION=6

kwin_pid= registry_pid= monitor_pid=
cleanup() {
  for pid in "$kwin_pid" "$registry_pid" "$monitor_pid"; do
    if [[ -n "$pid" ]]; then
      kill -TERM "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
}
trap cleanup EXIT
registry=/usr/libexec/at-spi2-registryd
[[ -x "$registry" ]] || registry=/usr/lib/at-spi2-registryd
[[ -x "$registry" ]] || { echo 'AT-SPI registry unavailable' >&2; exit 1; }
"$registry" >"$session/registry.log" 2>&1 & registry_pid=$!
for attempt in {1..100}; do
  owner="$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.NameHasOwner org.a11y.atspi.Registry 2>/dev/null || true)"
  [[ "$owner" == *true* ]] && break
  sleep .05
done
[[ "$owner" == *true* ]] || { echo 'private AT-SPI registry unavailable' >&2; exit 1; }
gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
  --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled '<true>' >/dev/null
stdbuf -oL dbus-monitor --session "type='method_call',destination='org.freedesktop.portal.Desktop'" \
  "type='signal',interface='org.freedesktop.portal.Request',member='Response'" \
  "type='error'" \
  >"$session/portal-dbus.log" 2>&1 & monitor_pid=$!

# Qt 6.10's KDE theme reacts to an early portal ThemeChanged during KWin
# construction and crashes; only the private compositor uses the generic theme.
env -u WAYLAND_DISPLAY QT_QPA_PLATFORM=xcb QT_QPA_PLATFORMTHEME=generic \
  kwin_wayland --x11-display "$outer_display" --no-lockscreen \
  --socket="$WAYLAND_DISPLAY" --width 1024 --height 768 >"$session/kwin.log" 2>&1 & kwin_pid=$!
for attempt in {1..100}; do
  [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] && break
  if ! kill -0 "$kwin_pid" 2>/dev/null; then
    echo 'private KWin Wayland exited before its socket was ready' >&2; exit 1
  fi
  sleep .05
done
[[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] || {
  echo 'private KWin Wayland socket did not become ready' >&2; exit 1;
}
# D-Bus activation inherited no Wayland socket when this private bus started.
# Publish it only after KWin is ready so the KDE portal joins this compositor.
dbus-update-activation-environment WAYLAND_DISPLAY QT_QPA_PLATFORM XDG_CURRENT_DESKTOP \
  XDG_SESSION_TYPE KDE_FULL_SESSION KDE_SESSION_VERSION QT_LINUX_ACCESSIBILITY_ALWAYS_ON
python3 scripts/check-launcher-portal-consent.py "$decision" "$session"
settings="$session/config/io.github.briolabs.Maestria.Launcher/launcher.toml"
if [[ "$decision" == allow ]]; then
  [[ -f "$settings" ]] || { echo 'approved shortcut preference vanished after launcher quit' >&2; exit 1; }
else
  [[ ! -e "$settings" ]] || { echo 'denied shortcut preference appeared after launcher quit' >&2; exit 1; }
fi

#!/usr/bin/env bash
set -euo pipefail

fail() { echo "installed launcher portal smoke failed: $*" >&2; exit 1; }
[[ $# -eq 1 && -d "$1" ]] || fail 'usage: smoke-launcher-portal-package.sh LAUNCHER_PACKAGE_DIRECTORY'
package_dir="$(realpath "$1")"
shopt -s nullglob
packages=("$package_dir"/*.deb)
[[ ${#packages[@]} -eq 1 ]] || fail 'expected exactly one launcher Debian artifact'
package_name=io-github-briolabs-sillage-launcher
[[ "$(dpkg-deb --field "${packages[0]}" Package)" == "$package_name" ]] || fail 'wrong launcher package'
expected="install ok installed|$(dpkg-deb --field "${packages[0]}" Version)|$(dpkg-deb --field "${packages[0]}" Architecture)"
installed="$(dpkg-query --show --showformat='${Status}|${Version}|${Architecture}' "$package_name")"
[[ "$installed" == "$expected" ]] || fail "installed package differs from same-run artifact: $installed"
[[ "$(dpkg-query --search /usr/bin/sillage-launcher)" == "$package_name: /usr/bin/sillage-launcher" ]] || fail 'launcher binary is not package-owned'
[[ -f /usr/share/applications/io.github.briolabs.Sillage.Launcher.desktop ]] || fail 'installed desktop identity is missing'
for component in sillage-search sillage-extension-worker; do
  [[ ! -e "/usr/bin/$component" ]] || fail "optional $component must not be installed"
  status="$(dpkg-query --show --showformat='${Status}' "io-github-briolabs-$component" 2>/dev/null || true)"
  [[ "$status" != 'install ok installed' ]] || fail "optional $component package must not be installed"
done
for tool in kwin_wayland xvfb-run dbus-run-session dbus-update-activation-environment xdotool gdbus; do
  command -v "$tool" >/dev/null || fail "required private display tool unavailable: $tool"
done

root="$(mktemp -d -t sillage-installed-portal.XXXXXX)"
trap 'rm -rf -- "$root"' EXIT
unset SILLAGE_LOCAL_PORTAL_LAUNCHER
for decision in allow deny; do
  session="$root/$decision"
  mkdir -p "$session"/{home,config,cache,data,state,runtime}
  chmod 700 "$session/runtime"
  export HOME="$session/home" XDG_CONFIG_HOME="$session/config" XDG_CACHE_HOME="$session/cache"
  export XDG_DATA_HOME="$session/data" XDG_STATE_HOME="$session/state" XDG_RUNTIME_DIR="$session/runtime"
  export XDG_DATA_DIRS=/usr/share XDG_CURRENT_DESKTOP=KDE XDG_SESSION_TYPE=wayland
  unset DBUS_SESSION_BUS_ADDRESS WAYLAND_DISPLAY AT_SPI_BUS_ADDRESS
  if ! timeout --kill-after=5s 90s xvfb-run -a -s '-screen 0 1280x800x24' \
    dbus-run-session -- bash scripts/smoke-launcher-portal-session.sh "$decision" "$session"; then
    for log in "$session"/*.log; do
      [[ -f "$log" ]] && { echo "=== $log ===" >&2; cat "$log" >&2; }
    done
    fail "private $decision session failed"
  fi
done

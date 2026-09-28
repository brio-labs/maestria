#!/usr/bin/env python3
"""Exercise the installed launcher against a real private KDE GlobalShortcuts portal."""

from collections import deque
import os
import re
from pathlib import Path
import subprocess
import sys
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi


def descendants(root):
    pending = deque([root])
    for _ in range(2048):
        if not pending:
            break
        node = pending.popleft()
        yield node
        try:
            pending.extend(node.get_child_at_index(index) for index in range(node.get_child_count()))
        except (AttributeError, RuntimeError):
            continue


def visible_button(name):
    for node in descendants(Atspi.get_desktop(0)):
        if node is None:
            continue
        try:
            if (node.get_role() == Atspi.Role.PUSH_BUTTON and node.get_name() == name
                    and node.get_state_set().contains(Atspi.StateType.SHOWING)):
                return node
        except (AttributeError, RuntimeError):
            continue
    return None


def capture_private_display(destination):
    try:
        os.environ["GDK_BACKEND"] = "x11"
        gi.require_version("Gdk", "3.0")
        from gi.repository import Gdk
        Gdk.init([])
        screen = Gdk.Screen.get_default()
        window = screen.get_root_window()
        pixels = Gdk.pixbuf_get_from_window(window, 0, 0, window.get_width(), window.get_height())
        if pixels is not None:
            destination.parent.mkdir(parents=True, exist_ok=True)
            pixels.savev(str(destination), "png", [], [])
    except (Exception, SystemExit) as error:
        print(f"Private screenshot unavailable: {error}", file=sys.stderr)


def portal_evidence(path):
    log = path.read_text(errors="replace")
    blocks = re.split(r"(?=^(?:signal|method call|method return|error) time=)", log, flags=re.M)
    responses = []
    binding_handle = None
    for block in blocks:
        if "interface=org.freedesktop.portal.GlobalShortcuts; member=BindShortcuts" in block:
            if 'string "activate-launcher"' not in block or 'string "CTRL+space"' not in block:
                raise AssertionError(f"wrong shortcut requested by installed launcher: {block}")
            handle = re.search(r'string "handle_token"\s+variant\s+string "([^"]+)"', block)
            if handle is not None:
                binding_handle = handle.group(1)
        if "interface=org.freedesktop.portal.Request; member=Response" in block:
            code = re.search(r"^\s+uint32 ([0-9]+)$", block, flags=re.M)
            if code is not None:
                responses.append((int(code.group(1)), block))
    return log, responses, binding_handle


def run(decision, session):
    Atspi.init()
    monitor = session / "portal-dbus.log"
    binary = os.environ.get("SILLAGE_LOCAL_PORTAL_LAUNCHER", "/usr/bin/maestria-launcher")
    settings = session / "config/io.github.briolabs.Maestria.Launcher/launcher.toml"
    launcher_log = (session / "launcher.log").open("w")
    launcher = subprocess.Popen([binary, "--activate"], stdout=launcher_log, stderr=subprocess.STDOUT)
    try:
        deadline = time.monotonic() + 12
        while time.monotonic() < deadline:
            if launcher.poll() is not None:
                raise AssertionError(f"launcher exited before first-run Setup: {launcher.returncode}")
            setup = visible_button("Set Up Shortcut")
            if setup is not None:
                component = setup.get_component_iface()
                if component is not None:
                    rect = Atspi.Component.get_extents(component, Atspi.CoordType.SCREEN)
                    print(f"LAUNCHER_SETUP_ATSPI_BOUNDS={rect.x},{rect.y},{rect.width},{rect.height}", flush=True)
                break
            time.sleep(.1)
        # Slint's Wayland tree may be absent or report an AT-SPI action that
        # returns success without activating Setup. Use only the owned Xvfb
        # pointer; KDE's actual portal response remains the acceptance proof.
        subprocess.run(["xdotool", "mousemove", "700", "221", "click", "1"], check=True)
        print("LAUNCHER_SETUP_USED_PRIVATE_X11_POINTER_NOT_ATSPI", flush=True)
        deadline = time.monotonic() + 30
        choice = "OK" if decision == "allow" else "Cancel"
        while time.monotonic() < deadline:
            log, responses, binding_handle = portal_evidence(monitor)
            if launcher.poll() is not None:
                raise AssertionError(f"launcher exited before KDE consent: {launcher.returncode}")
            created = ("interface=org.freedesktop.portal.GlobalShortcuts; member=CreateSession" in log
                       and binding_handle is not None
                       and any(code == 0 and 'string "session_handle"' in block for code, block in responses))
            if created:
                button = visible_button(choice)
                if button is not None:
                    action = button.get_action_iface()
                    if action is not None and Atspi.Action.do_action(action, 0):
                        print(f"KDE_DIALOG_ATSPI_ACTION={choice}", flush=True)
                        break
                # Some nested KDE portals render their Wayland dialog without
                # an AT-SPI application. These coordinates address only our
                # owned 1280x800 Xvfb display, not the host desktop.
                time.sleep(1)
                x = "617" if decision == "allow" else "707"
                subprocess.run(["xdotool", "mousemove", x, "652", "click", "1"], check=True)
                print(f"KDE_DIALOG_USED_PRIVATE_X11_POINTER_NOT_ATSPI={choice}", flush=True)
                break
            time.sleep(.1)
        else:
            raise AssertionError(f"KDE {choice} consent request did not reach the private portal: {log[-2600:]}")

        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            log, responses, binding_handle = portal_evidence(monitor)
            completed = [(code, block) for code, block in responses
                         if binding_handle is not None
                         and f"/{binding_handle};" in block.splitlines()[0]]
            if completed:
                code, block = completed[-1]
                expected = 0 if decision == "allow" else 1
                if code != expected:
                    raise AssertionError(f"KDE {decision} returned portal response {code}, expected {expected}")
                if decision == "allow":
                    if ('string "shortcuts"' not in block
                            or 'string "activate-launcher"' not in block
                            or 'string "Ctrl+Space"' not in block):
                        raise AssertionError(f"KDE did not grant activate-launcher: {block}")
                    if settings.is_file() and 'shortcutSetup = "requested"' in settings.read_text():
                        print("INSTALLED_PORTAL_CREATE_SESSION_RESPONSE=0_BIND_RESPONSE=0_APPROVED_AND_PERSISTED", flush=True)
                        return
                elif not settings.exists():
                    print("INSTALLED_PORTAL_CREATE_SESSION_RESPONSE=0_BIND_RESPONSE=1_DENIED_WITHOUT_SETTINGS", flush=True)
                    return
            time.sleep(.1)
        raise AssertionError(f"{decision} response did not produce expected launcher settings: {responses!r}")
    except Exception:
        capture_private_display(Path("target/launcher-portal-diagnostics") / f"{decision}.png")
        raise
    finally:
        if launcher.poll() is None:
            subprocess.run([binary, "--quit"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
            try:
                launcher.wait(timeout=5)
            except subprocess.TimeoutExpired:
                launcher.terminate()
                launcher.wait(timeout=5)
        launcher_log.close()


if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] not in ("allow", "deny"):
        raise SystemExit("usage: check-launcher-portal-consent.py allow|deny PRIVATE_SESSION_ROOT")
    decision, session = sys.argv[1], Path(sys.argv[2])
    run(decision, session)

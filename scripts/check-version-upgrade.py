#!/usr/bin/env python3
"""Exercise retained launcher settings and extension grants inside private Xvfb."""
from __future__ import annotations

import re
import subprocess
import sys
import time
from collections import deque
from pathlib import Path

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

if (
    len(sys.argv) != 5
    or sys.argv[1] not in {"defaults", "prepare", "verify", "revoke"}
    or sys.argv[2] not in {"legacy", "current"}
    or (sys.argv[1] == "defaults" and sys.argv[2] != "current")
):
    raise SystemExit("usage: check-version-upgrade.py defaults current PRIVATE_ROOT WINDOW_ID | prepare|verify|revoke legacy|current PRIVATE_ROOT WINDOW_ID")

phase = sys.argv[1]
profile = sys.argv[2]
root = Path(sys.argv[3])
window_id = sys.argv[4]
window_title = subprocess.run(
    ["xdotool", "getwindowname", window_id], check=True, capture_output=True, text=True, timeout=5
).stdout.strip()
# The pinned legacy package already displays "Sillage Launcher"; its Debian,
# executable, and XDG identities are checked separately by the shell harness.
if window_title != "Sillage Launcher":
    raise SystemExit(f"expected the private launcher window, found {window_title!r}")
extension_id = "dev.sillage.version-upgrade"
extension_name = "Version Upgrade Broker"
state_value = "private-extension-profile-state-retained-on-reinstall"
clipboard_value = "private-xvfb-crossname-profile-copy-proof"
package_path = root / "package"

Atspi.init()


def nodes():
    pending = deque([Atspi.get_desktop(0)])
    while pending:
        node = pending.popleft()
        yield node
        try:
            pending.extend(
                child
                for index in range(node.get_child_count())
                if (child := node.get_child_at_index(index)) is not None
            )
        except (AttributeError, RuntimeError):
            pass


def visible_labels():
    labels = []
    for node in nodes():
        try:
            label = node.get_name()
            if label:
                labels.append(label[:200])
        except (AttributeError, RuntimeError):
            pass
    return labels


def named(label: str):
    for node in nodes():
        try:
            if node.get_name() == label:
                return node
        except (AttributeError, RuntimeError):
            pass
    return None


def expect(label: str, seconds: float = 15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        node = named(label)
        if node is not None:
            return node
        time.sleep(0.04)
    raise RuntimeError(f"installed Slint did not expose {label!r}; visible={visible_labels()[:60]!r}")


def accessible_action(label: str):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        for node in nodes():
            try:
                if node.get_name() != label:
                    continue
                state = node.get_state_set()
                if not (
                    state.contains(Atspi.StateType.SHOWING)
                    and state.contains(Atspi.StateType.ENABLED)
                ):
                    continue
                iface = node.get_action_iface()
                if iface is not None and Atspi.Action.do_action(iface, 0):
                    return node
            except (AttributeError, RuntimeError):
                continue
        time.sleep(0.04)
    raise RuntimeError(f"installed Slint has no activatable AT-SPI action {label!r}; visible={visible_labels()[:60]!r}")


def text_value(node) -> str:
    iface = node.get_text_iface()
    if iface is None:
        return ""
    return Atspi.Text.get_text(iface, 0, Atspi.Text.get_character_count(iface))


def click_component(node):
    component = node.get_component_iface()
    bounds = component.get_extents(Atspi.CoordType.SCREEN) if component else None
    if bounds is None or bounds.width <= 0 or bounds.height <= 0:
        raise RuntimeError(f"private X11 input {node.get_name()!r} has no visible bounds")
    subprocess.run(
        ["xdotool", "mousemove", "--sync", str(bounds.x + max(4, bounds.width // 3)),
         str(bounds.y + max(4, bounds.height // 2)), "click", "1"],
        check=True, timeout=5,
    )


def click_and_type(node, text: str):
    click_component(node)
    subprocess.run(["xdotool", "type", "--clearmodifiers", "--delay", "1", text], check=True, timeout=10)
    print("VERSION_UPGRADE_PATH_ENTRY_USED_PRIVATE_X11_POINTER", flush=True)


def exact_text(label: str, expected: str):
    deadline = time.monotonic() + 5
    actual = ""
    while time.monotonic() < deadline:
        node = named(label)
        actual = text_value(node) if node is not None else ""
        if actual == expected:
            return
        time.sleep(0.04)
    raise RuntimeError(f"launcher field {label!r} contained {actual!r}, expected {expected!r}")


def expect_showing(label: str):
    node = expect(label)
    if not node.get_state_set().contains(Atspi.StateType.SHOWING):
        raise RuntimeError(f"launcher control {label!r} is exposed but not showing")
    return node


def assert_not_showing(label: str):
    for node in nodes():
        try:
            if node.get_name() != label:
                continue
            showing = node.get_state_set().contains(Atspi.StateType.SHOWING)
        except (AttributeError, RuntimeError):
            continue
        if showing:
            raise RuntimeError(f"launcher unexpectedly shows {label!r}")


def wait_for_checkbox_state(label: str, checked: bool):
    deadline = time.monotonic() + 5
    actual = None
    while time.monotonic() < deadline:
        node = named(label)
        if node is not None:
            actual = node.get_state_set().contains(Atspi.StateType.CHECKED)
            if actual == checked:
                return node
        time.sleep(0.04)
    raise RuntimeError(f"launcher checkbox {label!r} checked={actual}, expected {checked}")


def verify_settings():
    assert_not_showing("Set Up Shortcut")
    assert_not_showing("Not Now")
    accessible_action("Open launcher preferences")
    checkbox = expect_showing("Reduce interface motion")
    if not checkbox.get_state_set().contains(Atspi.StateType.CHECKED):
        raise RuntimeError("launcher did not load reduceMotion=true from its private XDG settings")
    expect_showing("Global shortcut accelerator")
    exact_text("Global shortcut accelerator", "Control+Space")
    expect_showing("System ✓")
    accessible_action("Close preferences")
    print(f"CROSSNAME_PROFILE_SETTINGS_VERIFIED profile={profile} phase={phase} reduce_motion=true shortcut=Control+Space shortcut_setup=deferred theme=system", flush=True)


def verify_fresh_defaults():
    expect_showing("Set Up Shortcut")
    expect_showing("Not Now")
    accessible_action("Open launcher preferences")
    checkbox = expect_showing("Reduce interface motion")
    if checkbox.get_state_set().contains(Atspi.StateType.CHECKED):
        raise RuntimeError("first Sillage launch inherited the seeded legacy reduceMotion=true setting")
    expect_showing("Global shortcut accelerator")
    exact_text("Global shortcut accelerator", "Control+Space")
    expect_showing("System ✓")
    accessible_action("Close preferences")
    print("CROSSNAME_PROFILE_DEFAULTS_VERIFIED profile=current phase=first-launch reduce_motion=false shortcut=Control+Space shortcut_setup=unconfigured theme=system seeded_legacy_reduce_motion=true seeded_legacy_shortcut_setup=deferred inherited_nondefault_values=false", flush=True)

    accessible_action("Not Now")
    assert_not_showing("Set Up Shortcut")
    assert_not_showing("Not Now")
    accessible_action("Open launcher preferences")
    checkbox = expect_showing("Reduce interface motion")
    if checkbox.get_state_set().contains(Atspi.StateType.CHECKED):
        raise RuntimeError("Sillage reduceMotion changed while deferring its default shortcut offer")
    click_component(checkbox)
    wait_for_checkbox_state("Reduce interface motion", True)
    expect_showing("Global shortcut accelerator")
    exact_text("Global shortcut accelerator", "Control+Space")
    expect_showing("System ✓")
    accessible_action("Save launcher preferences")
    expect_showing("Preferences saved.")
    wait_for_checkbox_state("Reduce interface motion", True)
    expect_showing("Global shortcut accelerator")
    exact_text("Global shortcut accelerator", "Control+Space")
    expect_showing("System ✓")
    accessible_action("Close preferences")
    assert_not_showing("Set Up Shortcut")
    assert_not_showing("Not Now")
    print("CROSSNAME_PROFILE_CONFIGURED_AFTER_DEFAULTS profile=current phase=first-launch reduce_motion=true shortcut=Control+Space shortcut_setup=deferred theme=system transition=preferences_ui", flush=True)


def extension_description(prefix: str) -> str:
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        for node in nodes():
            try:
                name = node.get_name()
            except (AttributeError, RuntimeError):
                continue
            if name and name.startswith(prefix):
                return name
        time.sleep(0.04)
    raise RuntimeError(f"extension permission review was not accessible; visible={visible_labels()[:60]!r}")


def verify_review_permissions(new_install: bool):
    if new_install:
        description = extension_description(f"New installation\nExtension ID: {extension_id}\n")
        if (
            f"Extension ID: {extension_id}\nVersion: 1.0.0" not in description
            or not re.search(r"SHA-256 package identity: [0-9a-f]{64}\b", description)
            or '{"type":"copy","formats":["text"]}' not in description
            or '{"type":"storage","scope":"extension"}' not in description
            or "Newly granted permissions:" not in description
            or "fileSearch" in description
        ):
            raise RuntimeError(f"install consent did not present only the requested copy/storage permissions: {description!r}")
        accessible_action("Approve exact extension identity and permissions, then install")
        print(f"CROSSNAME_PROFILE_EXTENSION_CONSENT_VERIFIED profile={profile} copy=text storage=extension file_search=not_granted", flush=True)
    accessible_action(f"Review permissions and details for {extension_name} 1.0.0")

def private_copy():
    button = expect("Run action Initialize private upgrade state" if phase == "prepare" else "Run action Verify retained private upgrade state")
    state = button.get_state_set()
    component = button.get_component_iface()
    bounds = component.get_extents(Atspi.CoordType.SCREEN) if component else None
    if not (
        state.contains(Atspi.StateType.SHOWING)
        and state.contains(Atspi.StateType.ENABLED)
        and bounds is not None
        and bounds.width > 0
        and bounds.height > 0
    ):
        raise RuntimeError("extension capability action is not visible and enabled")
    iface = button.get_action_iface()
    if iface is not None and Atspi.Action.do_action(iface, 0):
        print(f"CROSSNAME_PROFILE_EXTENSION_ACTION_USED_ATSPI profile={profile}", flush=True)
    else:
        subprocess.run(
            ["xdotool", "mousemove", str(bounds.x + bounds.width // 2),
             str(bounds.y + bounds.height // 2), "click", "1"],
            check=True, timeout=5,
        )
        print(f"CROSSNAME_PROFILE_EXTENSION_ACTION_USED_PRIVATE_X11_POINTER_NOT_ATSPI profile={profile}", flush=True)
    expected_title = (
        "UPGRADE_PRIVATE_STATE_INITIALIZED_AND_COPY_ALLOWED"
        if phase == "prepare"
        else "UPGRADE_PRIVATE_STATE_VERIFIED_AND_COPY_ALLOWED"
    )
    expect(expected_title, seconds=20)
    clipboard = subprocess.run(
        ["xclip", "-selection", "clipboard", "-o"],
        check=True, capture_output=True, text=True, timeout=5,
    ).stdout
    if clipboard != clipboard_value:
        raise RuntimeError(f"private Xvfb clipboard contained {clipboard!r}, not the authorized extension copy")
    print(f"CROSSNAME_PROFILE_EXTENSION_COPY_VERIFIED profile={profile} phase={phase} clipboard=private_xvfb", flush=True)


def install_and_seed():
    accessible_action("Manage installed extensions and permissions")
    entry = expect("Local extension package path")
    click_and_type(entry, str(package_path))
    exact_text("Local extension package path", str(package_path))
    accessible_action("Install extension from the entered local package path")
    expect("Extension installation permission review")
    expect(f"{extension_name} 1.0.0")
    verify_review_permissions(new_install=True)
    accessible_action("Run command Version upgrade state")
    expect("UPGRADE_PRIVATE_STATE_NOT_INITIALIZED")
    accessible_action("Select Initialize private upgrade state")
    private_copy()
    accessible_action("Return to installed extensions")
    print(f"CROSSNAME_PROFILE_EXTENSION_INITIALIZED profile={profile} worker=bubblewrap storage=private_xdg copy=private_xvfb", flush=True)


def verify_retained():
    accessible_action("Manage installed extensions and permissions")
    verify_review_permissions(new_install=False)
    accessible_action("Run command Version upgrade state")
    expect("UPGRADE_STATE_RETAINED_AND_UNGRANTED_FILE_SEARCH_DENIED")
    accessible_action("Select Verify retained private upgrade state")
    private_copy()
    accessible_action("Return to installed extensions")
    print(f"CROSSNAME_PROFILE_EXTENSION_RETAINED profile={profile} phase={phase} id={extension_id} storage={state_value} ungranted_file_search=denied copy_grant=active", flush=True)
def revoke_extension_grant():
    accessible_action("Manage installed extensions and permissions")
    accessible_action(f"Review permissions and details for {extension_name} 1.0.0")
    accessible_action("Return to installed extensions")
    accessible_action(f"Revoke permissions for {extension_name} 1.0.0")
    expect("Extension notice")
    accessible_action(f"Review permissions and details for {extension_name} 1.0.0")
    if named("Run command Version upgrade state") is not None:
        raise RuntimeError("revoked extension grant still exposes its executable command")
    print(f"CROSSNAME_PROFILE_EXTENSION_GRANT_REVOKED profile={profile} id={extension_id} command=unavailable", flush=True)




try:
    if phase == "defaults":
        verify_fresh_defaults()
    else:
        verify_settings()
        if phase == "prepare":
            install_and_seed()
        elif phase == "verify":
            verify_retained()
        else:
            revoke_extension_grant()
except Exception as error:
    print(f"CROSSNAME_PROFILE_GUI_FAILURE profile={profile} phase={phase}: {error}", file=sys.stderr, flush=True)
    raise

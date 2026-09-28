#!/usr/bin/env python3
"""Exercise the installed launcher and worker in a private X11 desktop session."""
from collections import deque
from pathlib import Path
import re
import subprocess
import sys
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

root, window = Path(sys.argv[1]), sys.argv[2]
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


def named(label):
    for node in nodes():
        try:
            if node.get_name() == label:
                return node
        except (AttributeError, RuntimeError):
            pass
    return None


def labels():
    result = []
    for node in nodes():
        try:
            label = node.get_name()
            if label:
                result.append(label[:160])
        except (AttributeError, RuntimeError):
            pass
    return result


def expect(label, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        node = named(label)
        if node is not None:
            return node
        time.sleep(0.04)
    raise RuntimeError(f"installed Slint did not expose {label!r}; visible={labels()[:55]!r}")


def action(label):
    node = expect(label)
    iface = node.get_action_iface()
    if iface is None or not Atspi.Action.do_action(iface, 0):
        raise RuntimeError(f"installed Slint rejected action {label!r}")


def exact_text(label, expected):
    for attempt in range(50):
        node = named(label)
        text = node.get_text_iface() if node is not None else None
        value = Atspi.Text.get_text(text, 0, Atspi.Text.get_character_count(text)) if text else ""
        if value == expected:
            return
        time.sleep(0.04)
    raise RuntimeError(f"installed Slint entry {label!r} contained {value!r}, not {expected!r}")


query_name = "Search applications, commands, files, or calculate"
for attempt in range(100):
    query = expect(query_name)
    if query.get_state_set().contains(Atspi.StateType.FOCUSED):
        break
    subprocess.run(["xdotool", "windowactivate", "--sync", window], check=True)
    time.sleep(0.04)
else:
    raise RuntimeError("installed Slint query entry did not receive private X11 focus")
subprocess.run(["xdotool", "type", "--clearmodifiers", "orchid lantern"], check=True)
exact_text(query_name, "orchid lantern")
expect("Open About Sillage and Slint attribution")
print("INSTALLED_LAUNCHER_NATIVE_QUERY_RESPONSIVE", flush=True)

action("Manage installed extensions and permissions")
entry = expect("Local extension package path")
component = entry.get_component_iface()
if component is None:
    raise RuntimeError("installed Slint package-path entry has no accessible bounds")
bounds = component.get_extents(Atspi.CoordType.SCREEN)
subprocess.run(
    ["xdotool", "mousemove", "--sync", str(bounds.x + max(4, bounds.width // 3)),
     str(bounds.y + max(4, bounds.height // 2)), "click", "1"],
    check=True,
)
subprocess.run(
    ["xdotool", "type", "--clearmodifiers", "--delay", "1", str(root / "package")],
    check=True,
)
exact_text("Local extension package path", str(root / "package"))
action("Install extension from the entered local package path")
expect("Extension installation permission review")
expect("Private Broker 1.0.0")
review = expect("Exact extension version, content identity and requested permissions")
review_text = review.get_text_iface()
if review_text is None:
    raise RuntimeError("installed Slint consent detail has no accessible text")
description = Atspi.Text.get_text(
    review_text, 0, Atspi.Text.get_character_count(review_text)
)
if (
    "Extension ID: dev.sillage.private-broker\nVersion: 1.0.0" not in description
    or not re.search(r"SHA-256 package identity: [0-9a-f]{64}\b", description)
    or "Requested permissions:\n{\"type\":\"copy\",\"formats\":[\"text\"]}" not in description
    or "Newly granted permissions:\n{\"type\":\"copy\",\"formats\":[\"text\"]}" not in description
    or "fileSearch" in description
):
    raise RuntimeError(f"installed Slint offered unexpected package/grants: {description!r}")
action("Approve exact extension identity and permissions, then install")
action("Review permissions and details for Private Broker 1.0.0")
action("Run command Greetings")
expect("Private Broker denied ungranted file search")
action("Select Private greeting")
action("Run action Copy private greeting")
expect("Broker authorized copy")
clipboard = subprocess.run(
    ["xclip", "-selection", "clipboard", "-o"],
    check=True, capture_output=True, text=True, timeout=5,
).stdout
if clipboard != "private Sillage broker token":
    raise RuntimeError(f"installed broker wrote unexpected private clipboard text: {clipboard!r}")
print("INSTALLED_BROKER_UNGRANTED_SEARCH_DENIED_AND_COPY_VERIFIED", flush=True)
action("Return to installed extensions")

subprocess.run(["/usr/bin/maestria-launcher", "--quit"], check=True, capture_output=True)
for attempt in range(100):
    if named("Sillage Launcher") is None:
        break
    time.sleep(0.05)
else:
    raise RuntimeError("installed resident launcher did not quit before grant-retention check")
with (root / "restarted.log").open("w", encoding="utf-8") as log:
    restarted = subprocess.Popen(
        ["/usr/bin/maestria-launcher", "--activate"],
        stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
    )
(root / "restarted.pid").write_text(str(restarted.pid), encoding="ascii")
for attempt in range(100):
    result = subprocess.run(
        ["xdotool", "search", "--onlyvisible", "--name", "^Sillage Launcher$"],
        capture_output=True, text=True,
    )
    if result.returncode == 0 and result.stdout.strip():
        break
    if restarted.poll() is not None:
        raise RuntimeError(f"installed launcher exited after restart: {restarted.returncode}")
    time.sleep(0.05)
else:
    raise RuntimeError("installed launcher did not reopen after restart")
window = result.stdout.splitlines()[0]
subprocess.run(["xdotool", "windowactivate", "--sync", window], check=True)
action("Manage installed extensions and permissions")
action("Review permissions and details for Private Broker 1.0.0")
action("Run command Greetings")
expect("Private Broker denied ungranted file search")
action("Select Private greeting")
action("Run action Copy private greeting")
expect("Broker authorized copy")
clipboard = subprocess.run(
    ["xclip", "-selection", "clipboard", "-o"],
    check=True, capture_output=True, text=True, timeout=5,
).stdout
if clipboard != "private Sillage broker token":
    raise RuntimeError(f"installed broker lost approved Copy grant after restart: {clipboard!r}")
print("INSTALLED_BROKER_GRANT_RETAINED_AFTER_RESTART", flush=True)
action("Return to installed extensions")
action("Revoke permissions for Private Broker 1.0.0")
expect("Extension notice")
action("Review permissions and details for Private Broker 1.0.0")
if named("Run command Greetings") is not None:
    raise RuntimeError("revoked installed extension still exposes an executable command")
print("INSTALLED_BROKER_REVOKED_COMMAND_UNAVAILABLE", flush=True)

#!/usr/bin/env python3
"""Activate the real GTK chooser's accessible Open action on its private bus."""
from collections import deque
import time
import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi


def descendants(root):
    pending = deque([root])
    visited = 0
    while pending and visited < 2048:
        node = pending.popleft()
        visited += 1
        yield node
        for index in range(node.get_child_count()):
            child = node.get_child_at_index(index)
            if child is not None:
                pending.append(child)


Atspi.init()
deadline = time.monotonic() + 5
while time.monotonic() < deadline:
    dialog = next((node for node in descendants(Atspi.get_desktop(0))
                   if node.get_name() == "Open a local file"), None)
    if dialog is not None:
        button = next((node for node in descendants(dialog)
                       if node.get_role() == Atspi.Role.PUSH_BUTTON and node.get_name() == "Open"), None)
        if button is not None and button.get_state_set().contains(Atspi.StateType.SENSITIVE):
            action = button.get_action_iface()
            if action is None or not action.do_action(0):
                raise RuntimeError("GTK did not accept the Open accessibility action")
            print("Native GTK Open action activated", flush=True)
            break
    time.sleep(0.05)
else:
    raise RuntimeError("The native chooser's enabled Open action was unavailable")

#!/usr/bin/env python3
"""Observe actual modifier locks on the isolated native display."""
import json
import gi

gi.require_version("Gdk", "3.0")
from gi.repository import Gdk

Gdk.init([])
keymap = Gdk.Keymap.get_for_display(Gdk.Display.get_default())
print(json.dumps({
    "caps": keymap.get_caps_lock_state(),
    "num": keymap.get_num_lock_state(),
    "scroll": keymap.get_scroll_lock_state(),
}))

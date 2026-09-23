#!/usr/bin/python3
"""Observe the native clipboard by pasting into a separate real GTK entry."""
import json
import sys
import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, GLib, Gtk

if "--read" in sys.argv:
    print(json.dumps(Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).wait_for_text()), flush=True)
    raise SystemExit(0)

window = Gtk.Window(title="Maestria clipboard acceptance")
entry = Gtk.Entry()
window.add(entry)
window.show_all()
entry.grab_focus()


def paste():
    entry.emit("paste-clipboard")
    return False


def focused(*_args):
    GLib.idle_add(paste)
    return False


def finish():
    print(json.dumps(entry.get_text()), flush=True)
    window.destroy()
    Gtk.main_quit()
    return False


entry.connect("changed", lambda _entry: GLib.idle_add(finish))
window.connect("focus-in-event", focused)
if window.is_active():
    GLib.idle_add(paste)
GLib.timeout_add(3000, finish)
Gtk.main()

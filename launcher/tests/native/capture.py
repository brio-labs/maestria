#!/usr/bin/env python3
"""Capture the owned X11 test window, including its native WebKit surface."""
import sys
import gi

gi.require_version("Gdk", "3.0")
gi.require_version("GdkX11", "3.0")
from gi.repository import Gdk, GdkX11

Gdk.init([])
display = Gdk.Display.get_default()
window = GdkX11.X11Window.foreign_new_for_display(display, int(sys.argv[1]))
if window is None:
    raise RuntimeError("The owned native test window is unavailable")
pixels = Gdk.pixbuf_get_from_window(window, 0, 0, window.get_width(), window.get_height())
if pixels is None:
    raise RuntimeError("The owned native test surface could not be captured")
pixels.savev(sys.argv[2], "png", [], [])

#!/usr/bin/env python3
"""Own a single competing Space accelerator on the private test X server."""
import ctypes
import os
import sys

if not os.environ.get("MAESTRIA_NATIVE_TEST_ROOT") or os.environ.get("GDK_BACKEND") != "x11":
    raise RuntimeError("A private native-test X11 environment is required")
mask = int(sys.argv[1])
if mask not in (4, 12):
    raise ValueError("Only the test Control/Control+Alt Space bindings are supported")
x11 = ctypes.CDLL("libX11.so.6")
x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
x11.XOpenDisplay.restype = ctypes.c_void_p
x11.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x11.XDefaultRootWindow.restype = ctypes.c_ulong
x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x11.XKeysymToKeycode.restype = ctypes.c_ubyte
x11.XGrabKey.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_uint, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_int]
x11.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
display = x11.XOpenDisplay(None)
if not display:
    raise RuntimeError("The private X11 display could not be opened")
try:
    root = x11.XDefaultRootWindow(display)
    key = x11.XKeysymToKeycode(display, 0x20)
    x11.XGrabKey(display, key, mask, root, 0, 1, 1)
    # XSync makes BadAccess fail before the readiness marker.
    x11.XSync(display, 0)
    print("SHORTCUT_GRABBED", flush=True)
    sys.stdin.readline()
finally:
    x11.XCloseDisplay(display)

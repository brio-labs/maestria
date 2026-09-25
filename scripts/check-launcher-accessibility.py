#!/usr/bin/env python3
"""Verify the native launcher exposes and accepts keyboard text via AT-SPI."""

import os
from pathlib import Path
from collections import deque
import subprocess
import sys
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

QUERY_NAME = "Search applications, commands, files, or calculate"
ABOUT_NAME = "Open About Sillage and Slint attribution"
WINDOW_NAME = "Sillage Launcher"
SETUP_NAME = "Set Up Shortcut"
DEFER_NAME = "Not Now"
RESET_NAME = "Reset launcher preferences to defaults"
CONFIRM_RESET_NAME = "Confirm resetting launcher preferences"


def descendants(root):
    pending = deque([root])
    visited = 0
    while pending and visited < 2048:
        node = pending.popleft()
        visited += 1
        yield node
        try:
            for index in range(node.get_child_count()):
                child = node.get_child_at_index(index)
                if child is not None:
                    pending.append(child)
        except (AttributeError, RuntimeError):
            continue


def query_text(node) -> str:
    text = node.get_text_iface()
    if text is None:
        return ""
    return Atspi.Text.get_text(text, 0, Atspi.Text.get_character_count(text))


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(
            "usage: check-launcher-accessibility.py EXPECTED_QUERY|--preferences|"
            "--offer|--setup-offer|--requested-shortcut|--defer-offer|--defer-offer-no-seat|--no-offer|"
            "--save-preferences|--conflict-rejected|--calculation|--application|"
            "--reset-cancel|--reset-begin|--reset-hidden|--reset-cleared"
        )
    expected_query = sys.argv[1]

    Atspi.init()
    deadline = time.monotonic() + 15
    window = None
    accelerator = None
    save = None
    observed_buttons = set()
    while time.monotonic() < deadline:
        try:
            nodes = list(descendants(Atspi.get_desktop(0)))
            role_nodes = []
            for node in nodes:
                try:
                    role_nodes.append((node, node.get_role(), node.get_name()))
                except (AttributeError, RuntimeError):
                    continue
            window = next(
                (node for node, _, name in role_nodes if name == WINDOW_NAME), None
            )
            buttons = {
                name: node
                for node, role, name in role_nodes
                if role == Atspi.Role.PUSH_BUTTON
            }
            observed_buttons = set(buttons)
            if expected_query == "--reset-cancel":
                reset = buttons.get(RESET_NAME)
                if reset is not None:
                    action = reset.get_action_iface()
                    if action is None or not Atspi.Action.do_action(action, 0):
                        raise SystemExit("AT-SPI could not open Reset confirmation")
                    expected_query = "--confirm-reset"
                time.sleep(0.05)
                continue
            if expected_query == "--confirm-reset":
                if buttons.get(CONFIRM_RESET_NAME) is not None:
                    subprocess.run(["xdotool", "key", "Escape"], check=True, timeout=5)
                    subprocess.run(
                        ["xdotool", "key", "ctrl+comma"], check=True, timeout=5
                    )
                    expected_query = "--reset-cancelled"
                time.sleep(0.05)
                continue
            if expected_query == "--reset-cancelled":
                if buttons.get(RESET_NAME) is not None:
                    print(
                        "AT-SPI confirmed closing Preferences cancels pending reset",
                        flush=True,
                    )
                    return
                time.sleep(0.05)
                continue
            if expected_query == "--reset-begin":
                reset = buttons.get(RESET_NAME)
                if reset is not None:
                    action = reset.get_action_iface()
                    if action is None or not Atspi.Action.do_action(action, 0):
                        raise SystemExit("AT-SPI could not open Reset confirmation")
                    expected_query = "--reset-begin-confirm"
                time.sleep(0.05)
                continue
            if expected_query == "--reset-begin-confirm":
                if buttons.get(CONFIRM_RESET_NAME) is not None:
                    print("AT-SPI opened Reset confirmation before native reactivation", flush=True)
                    return
                time.sleep(0.05)
                continue
            if expected_query == "--reset-cleared":
                if buttons.get(RESET_NAME) is not None:
                    print(
                        "AT-SPI confirmed native reactivation cancels pending Reset",
                        flush=True,
                    )
                    return
                time.sleep(0.05)
                continue
            if expected_query == "--reset-hidden":
                if RESET_NAME not in buttons and CONFIRM_RESET_NAME not in buttons:
                    print("AT-SPI confirmed reactivation closed Preferences", flush=True)
                    return
                time.sleep(0.05)
                continue
            if expected_query in {
                "--offer",
                "--setup-offer",
                "--requested-shortcut",
                "--defer-offer",
                "--defer-offer-no-seat",
                "--wait-deferred",
                "--wait-deferred-no-seat",
                "--wait-requested",
                "--no-offer",
            }:
                setup = buttons.get(SETUP_NAME)
                defer = buttons.get(DEFER_NAME)
                if window is None:
                    time.sleep(0.05)
                    continue
                if expected_query == "--offer" and setup is not None and defer is not None:
                    print(
                        "AT-SPI exposed the first-run shortcut setup and deferral buttons",
                        flush=True,
                    )
                    return
                if expected_query == "--setup-offer" and setup is not None and defer is not None:
                    action = setup.get_action_iface()
                    if action is None or not Atspi.Action.do_action(action, 0):
                        raise SystemExit("AT-SPI could not activate Set Up Shortcut")
                    expected_query = "--wait-requested"
                if expected_query in {"--wait-requested", "--requested-shortcut"}:
                    config_home = os.environ.get("XDG_CONFIG_HOME")
                    if not config_home:
                        raise SystemExit("XDG_CONFIG_HOME is required for shortcut setup smoke")
                    settings = (
                        Path(config_home)
                        / "io.github.briolabs.Maestria.Launcher"
                        / "launcher.toml"
                    )
                    if (
                        setup is None
                        and defer is None
                        and settings.is_file()
                        and 'shortcutSetup = "requested"' in settings.read_text()
                        and 'shortcut = "Control+Space"' in settings.read_text()
                    ):
                        print("AT-SPI confirmed native shortcut setup persisted and first-run offer closed", flush=True)
                        return
                    time.sleep(0.05)
                    continue
                if expected_query in {"--defer-offer", "--defer-offer-no-seat"} and setup is not None and defer is not None:
                    no_seat = expected_query == "--defer-offer-no-seat"
                    action = defer.get_action_iface()
                    if action is None or not Atspi.Action.do_action(action, 0):
                        raise SystemExit("AT-SPI could not activate Not Now")
                    expected_query = "--wait-deferred-no-seat" if no_seat else "--wait-deferred"
                if expected_query in {"--wait-deferred", "--wait-deferred-no-seat"} and setup is None and defer is None:
                    config_home = os.environ.get("XDG_CONFIG_HOME")
                    if not config_home:
                        raise SystemExit("XDG_CONFIG_HOME is required for deferral persistence smoke")
                    settings = (
                        Path(config_home)
                        / "io.github.briolabs.Maestria.Launcher"
                        / "launcher.toml"
                    )
                    query = next(
                        (
                            node
                            for node in nodes
                            if node.get_role() == Atspi.Role.ENTRY
                            and node.get_name() == QUERY_NAME
                        ),
                        None,
                    )
                    if (
                        settings.is_file()
                        and 'shortcutSetup = "deferred"' in settings.read_text()
                        and (
                            expected_query == "--wait-deferred-no-seat"
                            or (
                                query is not None
                                and query.get_state_set().contains(Atspi.StateType.FOCUSED)
                            )
                        )
                    ):
                        print(
                            "AT-SPI Not Now persisted deferral"
                            + (
                                " (focus untested without a compositor seat)"
                                if expected_query == "--wait-deferred-no-seat"
                                else " and restored search focus"
                            ),
                            flush=True,
                        )
                        return
                if expected_query == "--no-offer" and setup is None and defer is None:
                    print("AT-SPI confirmed persisted shortcut deferral has no first-run offer", flush=True)
                    return
                time.sleep(0.05)
                continue
            if expected_query == "--save-preferences":
                save = buttons.get("Save launcher preferences")
                if save is not None:
                    action = save.get_action_iface()
                    if action is None or not Atspi.Action.do_action(action, 0):
                        raise SystemExit("AT-SPI could not activate Save preferences")
                    print("AT-SPI invoked native Preferences Save action", flush=True)
                    return
                time.sleep(0.05)
                continue
            if expected_query == "--conflict-rejected":
                config_home = os.environ.get("XDG_CONFIG_HOME")
                if not config_home:
                    raise SystemExit("XDG_CONFIG_HOME is required for conflict-grab smoke")
                settings = (
                    Path(config_home)
                    / "io.github.briolabs.Maestria.Launcher"
                    / "launcher.toml"
                )
                for node, _, name in role_nodes:
                    try:
                        text = name + " " + query_text(node)
                    except (AttributeError, RuntimeError):
                        continue
                    if (
                        "could not be registered" in text.lower()
                        and settings.is_file()
                        and 'shortcut = "Control+Space"' in settings.read_text()
                        and 'shortcutSetup = "requested"' in settings.read_text()
                    ):
                        print("AT-SPI confirmed X11 conflicting grab rejected without changing saved shortcut", flush=True)
                        return
                time.sleep(0.05)
                continue
            if expected_query == "--calculation":
                if window is not None and any(
                    role == Atspi.Role.PUSH_BUTTON and name == "4 2 + 2"
                    for _, role, name in role_nodes
                ):
                    print("AT-SPI exposed the calculated 4 result for 2 + 2", flush=True)
                    return
                time.sleep(0.05)
                continue
            if expected_query == "--application":
                if window is not None and any(
                    role == Atspi.Role.PUSH_BUTTON
                    and name.startswith("Sillage Package Smoke Application")
                    for _, role, name in role_nodes
                ):
                    print("AT-SPI exposed isolated native desktop entry result", flush=True)
                    return
                time.sleep(0.05)
                continue
            if expected_query == "--preferences":
                accelerator = next(
                    (
                        node
                        for node, role, name in role_nodes
                        if role == Atspi.Role.ENTRY
                        and name == "Global shortcut accelerator"
                    ),
                    None,
                )
                save = buttons.get("Save launcher preferences")
                if (
                    window is not None
                    and accelerator is not None
                    and save is not None
                    and accelerator.get_state_set().contains(Atspi.StateType.FOCUSED)
                ):
                    print("AT-SPI exposed focused keyboard-opened Preferences", flush=True)
                    return
                time.sleep(0.05)
                continue
            query = next(
                (
                    node
                    for node in nodes
                    if node.get_role() == Atspi.Role.ENTRY
                    and node.get_name() == QUERY_NAME
                ),
                None,
            )
            about = next(
                (
                    node
                    for node in nodes
                    if node.get_role() == Atspi.Role.PUSH_BUTTON
                    and node.get_name() == ABOUT_NAME
                ),
                None,
            )
            if window is not None and query is not None and about is not None:
                actual_query = query_text(query)
                if (
                    actual_query == expected_query
                    and query.get_state_set().contains(Atspi.StateType.FOCUSED)
                ):
                    print(
                        "AT-SPI exposed the launcher window, named query entry, "
                        "keyboard input, and accessible About button",
                        flush=True,
                    )
                    return
        except (AttributeError, RuntimeError):
            pass
        time.sleep(0.05)

    if expected_query == "--offer":
        print(
            f"Shortcut-offer diagnostic: window={window is not None}, "
            f"setup={SETUP_NAME in observed_buttons}, defer={DEFER_NAME in observed_buttons}, "
            f"buttons={sorted(observed_buttons)}",
            file=sys.stderr,
        )
    if expected_query == "--preferences":
        print(
            f"Preferences diagnostic: window={window is not None}, "
            f"accelerator={accelerator is not None}, save={save is not None}, "
            f"focused={accelerator is not None and accelerator.get_state_set().contains(Atspi.StateType.FOCUSED)}",
            file=sys.stderr,
        )
    raise SystemExit(
        "native window did not expose focused Preferences accelerator"
        if expected_query == "--preferences"
        else "native Reset confirmation did not return to its prior state"
        if expected_query.startswith("--reset")
        else "native first-run shortcut offer state did not become accessible or persist"
        if expected_query.startswith("--")
        else "native window did not expose focused AT-SPI search input with the "
        "typed query and accessible Slint About control"
    )


if __name__ == "__main__":
    main()

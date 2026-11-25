#!/usr/bin/env python3
"""Open, focus or close this workspace's docket overlay."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

PLUGIN = "nicococo.docket"
ENTRYPOINT = "dashboard"


def run(*args):
    result = subprocess.run([os.environ.get("HERDR_BIN_PATH", "herdr"), *args],
                            capture_output=True, text=True, check=True)
    return json.loads(result.stdout)["result"]


def toggle(state_dir, workspace, socket):
    if not workspace or not socket:
        raise ValueError("Herdr workspace or socket context is missing")
    state_dir.mkdir(parents=True, exist_ok=True)
    key = hashlib.sha256((socket + "\0" + workspace).encode()).hexdigest()[:24]
    state_path = state_dir / (key + ".json")
    with (state_dir / (key + ".lock")).open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        panes = run("pane", "list", "--workspace", workspace)["panes"]
        try:
            saved = json.loads(state_path.read_text())
        except (FileNotFoundError, json.JSONDecodeError):
            saved = {}
        owned = next((p for p in panes if p.get("pane_id") == saved.get("pane_id")
                      and p.get("terminal_id") == saved.get("terminal_id")
                      and p.get("workspace_id") == workspace), None)
        if owned:
            action = "close" if owned.get("focused") else "focus"
            run("plugin", "pane", action, owned["pane_id"])
            if action == "close":
                state_path.unlink(missing_ok=True)
            return
        opened = run("plugin", "pane", "open", "--plugin", PLUGIN,
                     "--entrypoint", ENTRYPOINT, "--placement", "overlay", "--focus")
        plugin_pane = opened["plugin_pane"]
        pane = plugin_pane["pane"]
        if (plugin_pane["plugin_id"] != PLUGIN or plugin_pane["entrypoint"] != ENTRYPOINT
                or pane["workspace_id"] != workspace):
            raise ValueError("Unexpected Herdr plugin pane response")
        state_path.write_text(json.dumps({"pane_id": pane["pane_id"],
                                          "terminal_id": pane["terminal_id"]}))


def main():
    try:
        toggle(Path(os.environ["HERDR_PLUGIN_STATE_DIR"]),
               os.environ.get("HERDR_WORKSPACE_ID"), os.environ.get("HERDR_SOCKET_PATH"))
    except (KeyError, ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"docket: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

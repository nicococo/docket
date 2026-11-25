#!/usr/bin/env python3
"""Install docket and add a Herdr shortcut when the key is available."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib

KEY = "prefix+d"
ACTION = "nicococo.docket.toggle"


def values(value):
    if isinstance(value, dict):
        for child in value.values():
            yield from values(child)
    elif isinstance(value, list):
        for child in value:
            yield from values(child)
    elif isinstance(value, str):
        yield value.lower()


def register(path, defaults):
    old = path.read_text() if path.exists() else ""
    config = tomllib.loads(old)
    commands = config.get("keys", {}).get("command", [])
    if any(c.get("command") == ACTION for c in commands):
        return "Docket shortcut already configured."
    if KEY in set(values(config.get("keys", {}))) or KEY in defaults.lower():
        return f"{KEY} is already in use; add a different shortcut manually (see README)."
    addition = ('\n[[keys.command]]\nkey = "prefix+d"\ntype = "plugin_action"\n'
                'command = "nicococo.docket.toggle"\ndescription = "Toggle docket"\n')
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=".docket-", dir=path.parent)
    try:
        os.fchmod(fd, path.stat().st_mode & 0o777 if path.exists() else 0o600)
        with os.fdopen(fd, "w") as file:
            file.write(old.rstrip() + "\n" + addition)
            file.flush()
            os.fsync(file.fileno())
        if (path.read_text() if path.exists() else "") != old:
            raise RuntimeError("Herdr config changed; shortcut was not added")
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)
    return "Added prefix+d. Run 'herdr server reload-config' in Herdr."


def main():
    subprocess.run(["herdr", "plugin", "install", "nicococo/docket"], check=True)
    default = subprocess.check_output(["herdr", "--default-config"], text=True)
    config_home = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    path = Path(os.environ.get("HERDR_CONFIG_PATH", config_home / "herdr/config.toml"))
    print(register(path, default))


if __name__ == "__main__":
    try:
        main()
    except (OSError, subprocess.CalledProcessError, ValueError, RuntimeError) as error:
        print(f"docket installer: {error}", file=sys.stderr)
        sys.exit(1)

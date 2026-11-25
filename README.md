# docket

[![CI](https://github.com/nicococo/docket/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/nicococo/docket/actions/workflows/ci.yml) [![Coverage](https://codecov.io/gh/nicococo/docket/graph/badge.svg)](https://codecov.io/gh/nicococo/docket) [![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

Calendar, feeds, notes and email in one keyboard-driven terminal dashboard.

![Illustrated mockup of the docket dashboard](assets/docket-mockup.png)

*Illustrated mockup; content and colors depend on your terminal and data.*

## Install in Herdr

Requires Herdr 0.9+, Rust, Cargo and Python 3 on Linux or macOS:

```sh
herdr plugin install nicococo/docket
```

Herdr builds docket on installation. From a local checkout, `python3 scripts/install-herdr.py` installs the plugin and adds `prefix+d` if the key is free (Python 3.11+). Otherwise, to open it, run:

```sh
herdr plugin action invoke nicococo.docket.toggle
```

For a shortcut, add this to `~/.config/herdr/config.toml`, choosing an unused key:

```toml
[[keys.command]]
key = "prefix+d"
type = "plugin_action"
command = "nicococo.docket.toggle"
description = "Toggle docket"
```

Reload with `herdr server reload-config`. The shortcut opens the dashboard, focuses it if already open, and closes it when focused. Each workspace has its own overlay.

### Update or remove

```sh
herdr plugin update nicococo.docket
herdr plugin uninstall nicococo/docket
```

Close and reopen the dashboard after an update. Uninstalling preserves your docket data; remove the shortcut from Herdr config if you added it.

For a locally linked checkout, pull your changes and run `cargo build --release --locked` in that checkout instead. Close and reopen the dashboard; linking does not build the binary.

## Standalone

```sh
cargo install --path . --locked
docket
```

Docket creates its default configuration on first run. Press `?` for keys, `Tab` to switch panes, `z` to zoom, and `Ctrl+G` for settings. Use `Tab` to choose a settings page, arrows to select, `Enter` to edit, `Ctrl+S` to save, and `Esc` to close. On Herdr, the Herdr page adds or removes shortcuts and updates the plugin; shortcut changes apply when saved.

## Data and setup

Settings live in `~/.config/docket/config.toml`; notes are Markdown files under `~/.config/docket/notes/` by default. Calendar, feed and email connections are optional. Credentials are stored separately in `~/.config/docket/credentials/`. See [setup instructions](INSTRUCTIONS.md).

There is no telemetry or docket-hosted service. Docket contacts the feeds, calendars and mail servers you configure.

## Development

```sh
cargo test --features widgets-all
python3 -m unittest discover -s tests -p 'test_herdr*.py'
```

CI uploads an LCOV report as an artifact on every run. The coverage badge uses
Codecov; enable this repository in Codecov to publish the main-branch report.

Docket is GPL-3.0-or-later and is a fork of [glint](https://github.com/ntrospect0/glint) by ntrospect0. See [LICENSE](LICENSE) and [CONTRIBUTING.md](CONTRIBUTING.md).

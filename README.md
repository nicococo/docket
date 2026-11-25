# docket

Calendar, feeds, notes and email in one keyboard-driven terminal dashboard.

![Illustrated mockup of the docket dashboard](assets/docket-mockup.png)

*Illustrated mockup; content and colors depend on your terminal and data.*

## Install in Herdr

Requires Herdr 0.9+, Rust, Cargo and Python 3 on Linux or macOS:

```sh
herdr plugin install nicococo/docket
```

Herdr builds docket on installation. To open it, run:

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

Reload with `herdr server reload-config`. The shortcut opens the dashboard, focuses it if already open, and closes it when focused. Each workspace has its own overlay. Update with `herdr plugin update nicococo.docket`.

For a local checkout, run `cargo build --release --locked` and `herdr plugin link /path/to/docket`. Linking does not build the binary.

## Standalone

```sh
cargo install --path . --locked
docket
```

Docket creates its default configuration on first run. Press `?` for keys, `Tab` to switch panes, `z` to zoom, and `:settings` to edit configuration. Save settings with `Ctrl+S` or cancel with `Esc`.

## Data and setup

Settings live in `~/.config/docket/config.toml`; notes are Markdown files under `~/.config/docket/notes/` by default. Calendar, feed and email connections are optional. Credentials are stored separately in `~/.config/docket/credentials/`. See [setup instructions](INSTRUCTIONS.md).

There is no telemetry or docket-hosted service. Docket contacts the feeds, calendars and mail servers you configure.

## Development

```sh
cargo test --features widgets-all
python3 -m unittest discover -s tests -p 'test_herdr_toggle.py'
```

Docket is GPL-3.0-or-later and is a fork of [glint](https://github.com/ntrospect0/glint) by ntrospect0. See [LICENSE](LICENSE) and [CONTRIBUTING.md](CONTRIBUTING.md).

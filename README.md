# docket

[![CI](https://github.com/nicococo/docket/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/nicococo/docket/actions/workflows/ci.yml) [![Coverage](https://codecov.io/gh/nicococo/docket/graph/badge.svg)](https://codecov.io/gh/nicococo/docket) [![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

**A quick-glance cockpit and shared memory for people and agents.** Catch up on the working moment, capture the next step, and carry Markdown notes across projects and machines.

![Illustrated mockup of the docket dashboard](assets/docket-mockup.png)

*Illustrated mockup; content and colors depend on your terminal and data.*

## What docket is

Docket is a fixed four-pane terminal dashboard for quick check-ins, not a long-term email client or a configurable workspace. Scan unread and recent mail, glance at today’s calendars, skim selected feeds, and capture a note. Built for Herdr, it gives people and agents a shared place to catch up and write things down. Point its Markdown notes at a Git-synced folder to carry project context between computers.

## The panes

- **Calendar** — See today across local, CalDAV, and ICS calendars without leaving the terminal.
- **Feeds** — Scan selected RSS and Atom sources for what’s new.
- **Notes** — Keep quick Markdown notes and checklists. Board notes render as a Kanban view and remain ordinary Markdown.
- **Email** — Triage unread mail and messages from the last *N* days: mark read or unread, or delete in bulk.

## Install

Requires Herdr 0.9+, Rust, Cargo, and Python 3 on Linux or macOS:

```sh
herdr plugin install nicococo/docket
```

Herdr builds docket and registers `prefix+i` when the key is free. If it is already in use, add an unused key to `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+i"
type = "plugin_action"
command = "nicococo.docket.toggle"
description = "Toggle docket"
```

Reload with `herdr server reload-config`. Update with `herdr plugin update nicococo.docket`; remove with `herdr plugin uninstall nicococo/docket`. Uninstalling preserves docket data.

For standalone use:

```sh
cargo install --path . --locked
docket
```

Press `?` for keys, `Tab` to switch panes, `z` to zoom, and `s` for settings. See [INSTRUCTIONS.md](INSTRUCTIONS.md) for source setup.

## Set up sources with an agent

Ask an agent to read the [setup guide](INSTRUCTIONS.md) and help configure the sources you want. For example:

> Read the docket setup guide at https://github.com/nicococo/docket/blob/main/INSTRUCTIONS.md. Help me configure Gmail and Google Calendar in docket. First check which docket CLI is installed and preserve existing sources. Ask for setup details one at a time. Never ask me to paste a password or private calendar URL into chat; give me commands that prompt for secrets locally. Verify the sources load before saying setup is complete.

Agents can add named sources with `docket setup feed`, `docket setup email`, and `docket setup calendar`. Herdr installs the CLI inside its managed plugin directory rather than on `PATH`; the guide explains how to locate it. Secrets are read from stdin and stored in private credential files.

## Notes for people and agents

Notes are plain Markdown in the folder set under `[notes].notes_dir` (by default `~/.config/docket/notes/`). Docket’s `>` title paths appear as groups in the notes pane; board notes use a marker and Markdown task lists. The files remain editable in Obsidian and other Markdown tools.

To sync notes across machines, point `notes_dir` at a folder in a **private GitHub repository** or Obsidian vault and sync that folder with Git. Docket does not sync or merge files itself. Avoid editing the same note on two machines at once; resolve Git conflicts before continuing. Never put passwords, tokens, or private calendar URLs in synced notes.

Agents can use shared notes with `docket agent notes list|read|add`. For more structured memory, `docket agent memory` scopes notes by project, role, and stable agent label. Run `docket agent memory id` to see the resolved identity and path. Memory titles follow `Agents > project > role > agent > title`, keeping them separate from human notes and other agents while using the same Markdown store. The project defaults to the current Git root (or current directory outside Git); set `DOCKET_AGENT_ROLE` and `DOCKET_AGENT_ID`, or pass `--role` and `--agent`, for stable separate namespaces.

Example:

```sh
docket agent memory id --role implementer --agent omp
printf 'Release workflow\n\nUse the locked release build.\n' | docket agent memory add --role implementer --agent omp
docket agent memory list --role implementer --agent omp
```

Memory is a lightweight handoff: agents read relevant notes before work, then add or edit durable decisions, project constraints, discoveries, or next steps. People can review and correct them in Obsidian or the docket notes pane. Docket lists and filters the Markdown files directly; there is no separate database.

Agents can also read cached data with `docket agent calendar list`, `docket agent feeds list`, and `docket agent email list`; `docket agent email read <id>` prints a message body without changing its read state.

Docket has no telemetry or hosted service. It contacts only the feeds, calendar servers, and mail servers you configure.

## Development

```sh
cargo test --features widgets-all
python3 -m unittest discover -s tests -p 'test_herdr*.py'
```

Docket is GPL-3.0-or-later and a fork of [glint](https://github.com/ntrospect0/glint). See [LICENSE](LICENSE) and [CONTRIBUTING.md](CONTRIBUTING.md).

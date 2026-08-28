# docket

A fast, keyboard-driven terminal dashboard for getting things done —
calendar, email, notes (with a built-in kanban board), news, system
resources, and single-source RSS feeds, all in one grid you compose
yourself. Written in Rust with [ratatui](https://ratatui.rs).

![docket dashboard: calendar, a kanban notes board, and an email inbox](assets/screenshot.png)

No accounts, no telemetry, no OAuth. Everything is plain TOML under
`~/.config/docket/` — hand-edit it, back it up, sync it with dotfiles,
whatever you'd do with any other config.

```sh
git clone https://github.com/nicococo/docket.git && cd docket
make install PREFIX=~/.local   # or: sudo make install
docket                          # first run seeds a working default layout
```

---

## Highlights

- **Composable layout** — a grid of cells; any cell can hold a single
  widget or a **stack** you cycle between with `.` / `,`. Run the same
  widget kind multiple times (`calendar@work` + `calendar@personal`).
- **Notes with a kanban board** — any note toggles (`t`) between plain
  markdown and a colourful board view, columns from `## Heading`
  lines, cards from `- [ ]` items — still just a `.md` file underneath.
- **Email with AI actions** — `Enter` pops the full message with
  one-key summarize / explain / extract. Extracted todos and dates
  are selectable — add them straight to Notes or Calendar with `space`.
- **Live config reload** — edit any widget's TOML, see it picked up
  with no restart.
- **Nine bundled colour schemes**, live-switchable (`:scheme nord`),
  plus per-widget colour overrides.
- **Focus Zoom** (`z`) — enlarge the focused widget over a dimmed
  backdrop; every widget paints a richer view when it has the room.
- **No wizard, no cloud** — `docket --init` seeds default TOML
  directly; hand-edit credentials under `credentials/` (0600 perms).
  Nothing phones home that isn't in the [widget catalogue](#widget-catalogue) below.

---

## Install

Requires Rust 1.81+ via [`rustup`](https://rustup.rs/).

```sh
git clone https://github.com/nicococo/docket.git docket && cd docket
make install PREFIX=~/.local    # per-user, no sudo
# or: sudo make install         # system-wide
export PATH="$HOME/.local/bin:$PATH"   # if not already on $PATH
docket --version
```

| target | what it does |
|---|---|
| `make` / `make release` | release build at `target/release/docket` |
| `make install` | release build + copy to `$(PREFIX)/bin/docket` |
| `make test` | run the test suite |
| `make uninstall` / `make clean` | remove the binary / `cargo clean` |

**Slim builds** — every widget is its own Cargo feature; the default
`widgets-all` turns them all on:

```sh
cargo install --path . --no-default-features --features widget-calendar,widget-news
```

Available: `widget-calendar`, `widget-news`, `widget-feeds`,
`widget-email`, `widget-notes`, `widget-resources`.

**Updating**: `git pull && make install PREFIX=~/.local`.

**macOS**: `./assets/icon/install-macos-app.sh` wraps docket in a
double-clickable `~/Applications/Docket.app` (auto-detects your
terminal; force one with `TERMINAL=alacritty …`).

---

## Quickstart

First launch writes `~/.config/docket/config.toml` (a starter
calendar + news layout) plus a default TOML per widget kind. From there:

1. Hand-edit `config.toml`'s `[layout]` to add/remove/rearrange panes.
2. Edit each widget's own TOML (`calendar.toml`, `news.toml`, …) for
   feeds, providers, folders.
3. Fill in credentials where needed — templates are seeded under
   `credentials/`; no OAuth, just TOML. See
   [INSTRUCTIONS.md](INSTRUCTIONS.md) for the walkthrough per provider.

`docket --init` re-runs the seeding step any time (idempotent — only
fills in what's missing). Press `?` while running for the full
keybinding overlay.

---

## Widget catalogue

| widget | what it does | external services |
|---|---|---|
| **Calendar** | day / week / month views with event agenda | CalDAV (iCloud / Fastmail / Nextcloud), ICS/webcal feed, local TOML events |
| **Notes** | vim-flavoured multi-note editor with undo/redo; toggle any note into a kanban board; `[[Note Name]]` links | none — plain `.md` files |
| **Email** | unified inbox; `Enter` for a popup with AI summarize/explain/extract, extracted items addable to Notes/Calendar | any IMAP server (app password); LLM provider for AI actions |
| **News** | RSS/Atom aggregator with topic filters, keyword search, optional LLM summaries | any RSS/Atom feed; LLM provider |
| **Feeds** | tabbed single-source RSS reader, one tab per source | any RSS/Atom feed; LLM provider |
| **Resources** | htop-style CPU / memory / top-process view | local only |

Turn a kind off with `--no-default-features` (see [Slim
builds](#install)), or just leave it out of `[layout]`.

---

## Configuration

Everything lives under `~/.config/docket/`:

| file | what it controls |
|---|---|
| `config.toml` | theme, mouse-scroll direction, grid layout + cell placements |
| `colorschemes.toml` | theme palettes (`default`, `gruvbox`, `nord`, `tokyonight`, …) |
| `calendar.toml` / `email.toml` / `news.toml` / `feeds.toml` / `notes.toml` / `resources.toml` | per-widget settings |
| `llm.toml` | active LLM provider, model, rate limit |
| `credentials/` | API keys + app passwords, 0600 perms |
| `notes/<instance>/` | one `.md` file per note |

Hand-edit anything — the config watcher picks up changes live, no
`:reload` needed. Any widget's TOML can also carry a `[colors]` block
to override the active theme just for that widget:

```toml
# calendar.toml
[colors]
border.focused = { fg = "#e07b00", modifiers = ["bold"] }
```

> ⚠️ Dotted keys must be **unquoted** — `border.focused`, not `"border.focused"`.

---

## Keybindings

| key | action |
|---|---|
| `Tab` / `Shift+Tab` | cycle focused widget |
| `Shift+<letter>` | jump to a widget by its shortcut letter |
| `z` / `Esc` | zoom the focused widget in / out |
| `.` / `,` | rotate the active widget in a stack pane |
| `:` | command bar (`:scheme <name>`, `:reload`, `:news <terms>`) |
| `?` | full keybinding overlay (per-widget keys included) |
| `q` / `Ctrl+C` | quit |

Every widget has its own keys on top of these — press `?` any time to
see them for whatever's focused.

---

## CLI reference

```sh
docket                     # launch (seeds defaults on first run)
docket --init               # create/refresh ~/.config/docket/
docket --clear-cache [TARGET]   # wipe ~/.cache/docket/, or scope to a widget/instance
docket --config <FILE>      # override the default config location
docket --version
```

---

## Data & privacy

| where | what |
|---|---|
| `~/.config/docket/*.toml` | your config |
| `~/.config/docket/credentials/` (0600) | API keys, app passwords |
| `~/.config/docket/notes/` | notes as plain `.md` |
| `~/.cache/docket/` | regenerable caches; swept after 30 days |
| `~/.config/docket/docket.log` | runtime log |

docket never sends data anywhere not named in the [widget
catalogue](#widget-catalogue)'s external-services column — no
telemetry, no OAuth, no docket-owned backend.

Credentials are plain TOML, 0600, atomic writes — same convention as
`aws`/`gcloud`/`gh`/`ssh`. That protects against other local users and
offline disk theft, not against anything running as *your* user or a
backup tool sweeping `~/.config/`. Exclude `credentials/` from backups
if that matters to you, and prefer app passwords over master passwords.

---

## Troubleshooting

- **`docket` not found** — make sure `$(PREFIX)/bin` is on `$PATH`.
- **Feed images look chunky** — your terminal doesn't speak iTerm2 /
  Kitty / Sixel graphics; docket falls back to unicode half-blocks.
- **A layout cell is empty / logs "unknown widget kind"** — that
  widget isn't compiled in (slim build) or doesn't exist.
- **Logs**: `tail -f ~/.config/docket/docket.log`.
- **Reset**: move aside `~/.config/docket/` and re-run `docket --init`.

---

## Contributing

`make test` should pass clean on `main`. Adding a widget is purely
additive — implement `Widget` under `src/widgets/<name>/`, declare a
`widget-<name>` feature, register it in `src/widgets/registry.rs`. See
[CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md) for the
full picture. Issues and PRs welcome.

---

## Origins

docket is a hard fork of [**glint**](https://github.com/ntrospect0/glint)
by [**ntrospect0**](https://github.com/ntrospect0) — every widget
engine, the rendering pipeline, the config/cache architecture, is
ntrospect0's design. glint aims to be general-purpose: whatever mix of
stocks, weather, calendar, and news you want, composed your way.
docket exists because I wanted something narrower — opinionated around
my own work and project management, not a general-purpose canvas. So
rather than pile options onto glint, I forked it and pointed it one
direction. This is a standalone project now; it doesn't track glint's
upstream. If you want the general-purpose dashboard, go use glint —
this project wouldn't exist without it.

## License

GNU GPL v3 or later — see [LICENSE](LICENSE). You're free to use,
modify, and redistribute docket; any modified version you distribute
must stay GPL-licensed with copyright notices intact. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the contributor sign-off.

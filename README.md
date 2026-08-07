# docket

## Origins

docket is a hard fork of [**glint**](https://github.com/ntrospect0/glint),
a fast, keyboard-driven terminal dashboard written by
[**ntrospect0**](https://github.com/ntrospect0). Every widget engine,
the wizard, the rendering pipeline, the config/cache architecture —
the whole foundation this project stands on — is ntrospect0's design
and work. glint's own goal is to be a general-purpose, highly
adaptable dashboard: whatever mix of stocks, weather, calendar, news,
and more you want, composed your way. That idea, and the care put
into making it genuinely pleasant to use in a terminal, is what made
this fork worth doing in the first place — thank you, ntrospect0.

docket exists because I wanted something narrower and more opinionated:
a dashboard purpose-built around *my* work and project management, not
a general-purpose canvas. That's a different design goal than glint's,
not a better one, so rather than pile my opinions onto glint as
options and flags, I forked it and started shaping it toward that one
purpose. This is a standalone project going forward — it doesn't track
glint's upstream changes, and its own direction will diverge over
time. If you want the general-purpose dashboard, go use glint; it's
great, and this project wouldn't exist without it.

Like glint, docket is licensed GPL-3.0-or-later — see [`LICENSE`](LICENSE).

---

A fast, keyboard-driven terminal dashboard for getting things done.
Calendar, news, email, notes, system resources, and single-source RSS
feeds — all in one grid you compose yourself. Written in Rust with
[ratatui](https://ratatui.rs).

Everything is opt-in, locally configured, and persists in plain TOML
under `~/.config/docket/` — no accounts, no telemetry, no cloud
component docket controls, no OAuth. First launch seeds a working
default dashboard directly from TOML — no interactive setup flow to
walk through.

---

## Highlights

- **Six widget kinds**, each independently configurable — see the
  [widget catalogue](#widget-catalogue) below.
- **Composable layout**: a grid of cells; any cell can be a single
  widget or a **stack** of widgets you cycle between with `.` / `,`.
- **Multi-instance** — run the same widget kind in several panes
  (`calendar@work` + `calendar@personal`, `feeds@wsj` + `feeds@ai`).
- **Live config reload** — edit any widget's TOML and the dashboard
  picks it up without a restart.
- **Theming** — nine bundled colour schemes; per-widget colour overrides;
  add your own by editing one TOML file. `:scheme nord` switches live.
- **No setup wizard, no OAuth** — `docket --init` (or first launch
  with no config) seeds default TOML files directly; hand-edit them,
  and hand-edit credential TOML for CalDAV/ICS/IMAP. Every credential
  lives on disk under `~/.config/docket/credentials/` (0600 perms) —
  no cloud component, no browser handshake.
- **Focus Zoom** — press `z` to enlarge the focused widget into a
  centered frame over a dimmed backdrop, `z`/`Esc` to return. Given
  the room, every widget paints a richer **Full-tier** view.
- **Keyboard-first, mouse-friendly** — `Tab` cycles widgets,
  `Shift+<letter>` jumps to a widget by its shortcut letter, `:` opens
  a command bar, click anywhere to focus.

---

## Install

### From source (only option for now)

You need a recent Rust toolchain (1.81+) via [`rustup`](https://rustup.rs/).

```sh
git clone https://github.com/nicococo/docket.git docket
cd docket

# Per-user install (no sudo, installs to ~/.local/bin):
make install PREFIX=~/.local

# Or system-wide (typically needs sudo):
sudo make install
```

If `~/.local/bin` isn't on your `$PATH`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Verify: `docket --version`.

| target | what it does |
|---|---|
| `make` / `make release` | release build at `target/release/docket` |
| `make build` | debug build |
| `make install` | release build + copy to `$(PREFIX)/bin/docket` |
| `make uninstall` | remove `$(PREFIX)/bin/docket` |
| `make test` | run the test suite |
| `make clean` | `cargo clean` |

### Slim builds

Every widget compiles in only when its feature is enabled. The default
`widgets-all` umbrella turns them all on:

```sh
cargo install --path . --no-default-features \
  --features widget-calendar,widget-news
```

Available features: `widget-calendar`, `widget-news`, `widget-email`,
`widget-resources`, `widget-notes`, `widget-feeds`.

### Updating

```sh
git pull
make install PREFIX=~/.local   # or sudo make install
```

### macOS app icon (optional)

docket is a terminal program, but on macOS you can wrap it in a
double-clickable app that opens it in your terminal. With docket on
your `$PATH`: `./assets/icon/install-macos-app.sh` builds
`~/Applications/Docket.app`. Auto-detects Kitty, Ghostty, WezTerm,
Alacritty, Rio, iTerm2, Apple Terminal; force one with
`TERMINAL=alacritty ./assets/icon/install-macos-app.sh`.

---

## Quickstart

```sh
docket
# → "No config detected … writing defaults."
```

That writes `~/.config/docket/config.toml` (a two-pane calendar + news
layout) plus a default TOML for every widget kind. From there:

1. Hand-edit `config.toml`'s `[layout]` section to add, remove, or
   rearrange panes — see [Configuration](#configuration) below.
2. Edit the per-widget TOML (`calendar.toml`, `news.toml`, …) for
   RSS feeds, calendar providers, mailbox folders, etc.
3. Fill in credentials where a widget needs them — CalDAV/ICS for
   Calendar, IMAP for Email, an API key for LLM summaries. Templates
   are seeded under `~/.config/docket/credentials/`; no OAuth flow,
   just edit the TOML (see [INSTRUCTIONS.md](INSTRUCTIONS.md) for the
   walkthrough per provider).

`docket --init` re-runs the seeding step at any time — idempotent, it
only writes files that are still missing. Press `?` while running for
the live keybinding overlay.

---

## Widget catalogue

| widget | what it does | external services |
|---|---|---|
| **Calendar** | day / week / month views with event agenda | CalDAV (iCloud / Fastmail / Nextcloud), ICS/webcal feed (incl. Google Calendar's "secret address"), local TOML events |
| **News** | RSS / Atom aggregator with topic filters, keyword search (`:news <terms>`), optional per-article LLM summaries | any RSS/Atom feed; LLM provider for summaries |
| **Feeds** | tabbed single-source RSS reader (WSJ, MarketWatch, or any feed you point it at), one tab per source | any RSS/Atom feed; LLM provider for summaries |
| **Email** | unified inbox preview with optional per-message LLM summaries | any IMAP server (app password) |
| **Resources** | htop-style CPU / memory / top-process view | local `sysinfo` (no FFI) |
| **Notes** | vim-flavoured multi-note pad with undo/redo, per-note files | none — plain `.md` files under `~/.config/docket/notes/` |

Turn a kind off entirely with `--no-default-features` (see
[Slim builds](#slim-builds)), or just leave it out of `[layout]`.

---

## Configuration

All files live under `~/.config/docket/`:

| file | what it controls |
|---|---|
| `config.toml` | active colour scheme, mouse-scroll direction, status bar, grid layout, widget cell placements |
| `colorschemes.toml` | named theme palettes (`default`, `chalktone`, `gruvbox`, `tokyonight`, `rosepine`, `nord`, `bluloco`, `onedark`, `miasma`) |
| `news.toml` | RSS / Atom feeds, topic filters, LLM summary toggle, fetch-body strategy |
| `feeds.toml` (or `feeds@<instance>.toml`) | `[[feeds]]` blocks for one tabbed single-source reader instance |
| `calendar.toml` | CalDAV / ICS / Local providers + per-provider calendar IDs |
| `email.toml` | folders to follow, polling cadence, LLM-summary opt-in |
| `resources.toml` | refresh interval, top-N processes, sort key (CPU vs memory) |
| `notes.toml` | per-widget shortcut + colour overrides (notes themselves live under `notes/`) |
| `llm.toml` | active LLM provider (`anthropic` or `openai`), model, rate limit, cache size |
| `credentials/` | API keys + app passwords (`anthropic_key.toml`, `openai_key.toml`, `caldav.toml`, `ics.toml`, `imap.toml`) — 0600 perms |
| `notes/<instance>/` | one `.md` file per note, `mtime` sorts the list |

Most fields have sensible defaults; hand-edit any file and `:reload`
(or just save — the config watcher picks it up automatically).

### Layout example

```toml
# config.toml
version = 1

[global]
theme = "nord"

[layout]
columns = [28, 36, 36]
rows = [30, 35, 35]

[[layout.cells]]
widget = "calendar"
col = 0
row = 0

[[layout.cells]]
widget = "email"
col = 1
row = 0
col_span = 2              # span two columns

# Stack pane: three widgets share row 1, cols 1–2; rotate with . / ,
[[layout.cells]]
widgets = ["news", "feeds", "notes"]
col = 1
row = 1
col_span = 2

[[layout.cells]]
widget = "resources"
col = 0
row = 2
col_span = 2
```

### Multi-instance widgets

Cells can reference a widget as `kind@instance`:

```toml
[[layout.cells]]
widget = "feeds@wsj"
col = 0
row = 2

[[layout.cells]]
widget = "feeds@marketwatch"
col = 1
row = 2
```

The first reads `feeds@wsj.toml`, the second `feeds@marketwatch.toml` —
each instance is fully independent. Same trick works for calendars
(work + personal), email (two accounts), etc. A bare `widget = "feeds"`
(no `@instance`) reads the implicit `main` instance's `feeds.toml`.

### Per-widget colour overrides

Any widget's TOML can carry a `[colors]` block that overrides the
active theme just for that widget:

```toml
# calendar.toml
[colors]
border.focused = { fg = "#e07b00", modifiers = ["bold"] }
```

> ⚠️ Dotted keys must be **unquoted** (`border.focused`, not
> `"border.focused"`). Quoted dotted keys silently fail to deserialize.

---

## Keybindings

### Global

| key | action |
|---|---|
| `Tab` / `Shift+Tab` | cycle focused widget |
| `Shift+<letter>` | jump focus to a widget by its shortcut letter |
| `click cell` | focus that widget |
| `z` / `Shift+Z` | zoom the focused widget in / out |
| `Esc` | exit zoom (first clears the widget's own mode, e.g. Notes insert) |
| `.` / `,` | rotate the active widget in a stack pane |
| `:` | open the command bar |
| `?` | toggle help overlay |
| `q` / `Ctrl+C` | quit |

### Command bar (`:`)

| command | what it does |
|---|---|
| `:scheme <name>` | switch colour scheme (persists to `config.toml`) |
| `:reload` | re-read every widget's TOML without restarting |
| `:news <terms>` | filter News by keyword |
| `:feeds <terms>` | filter the active Feeds instance by keyword |

### Common per-widget keys

| widget | keys |
|---|---|
| **Calendar** | `d` / `w` / `m` day/week/month · `h` / `l` prev/next period · `←↑↓→` move selected day (month view) · `j` / `k` scroll agenda · `t` today · click a day to select it |
| **News** | `↑/↓` select · `←/→` filter tabs · `e` expand · `s` LLM summary · `Enter` open · `x` clear search |
| **Feeds** | `↑/↓` select · `←/→` topic tab · `e`/`Enter` expand · `o` open in browser · `s` LLM summary · `r` refresh · `x` clear search |
| **Email** | `↑/↓` select · `←/→` folder · `e`/`Enter` expand · `o` open in mail client · `s` LLM summary · `u` mark read/unread · `r` refresh |
| **Resources** | `m` toggle sort (CPU ↔ memory) · `r` force refresh |
| **Notes** | `+` new · `-` delete · `i` insert · `Esc` normal · `h`/`l` list / content · `j`/`k` scroll · `y` yank note · `Ctrl-Z`/`Ctrl-Shift-Z` undo/redo |

Hit `?` while running for the full overlay.

---

## CLI reference

```sh
docket                    # launch the dashboard (seeds defaults first, on first run)
docket --init              # create/refresh ~/.config/docket/ with default seed files
docket --clear-cache [TARGET]
                          # wipe ~/.cache/docket/ entirely, or scope to
                          # a widget kind (news) or instance (news@home)
docket --config <FILE>     # override the default XDG location
docket --version
```

---

## Data, privacy, and where things live

| where | what |
|---|---|
| `~/.config/docket/*.toml` | your config |
| `~/.config/docket/credentials/` (0600) | API keys, IMAP/CalDAV app passwords |
| `~/.config/docket/notes/` | notes as plain `.md` files |
| `~/.cache/docket/` | per-widget on-disk caches — regenerable; a startup sweep drops anything > 30 days old |
| `~/.config/docket/docket.log` | runtime log; `tail -f` it to debug |

docket never sends data anywhere not named in the widget catalogue's
external-services column. No telemetry, no OAuth, no docket-owned
backend.

Credentials are plain TOML with `0600` permissions and atomic writes —
same convention as `aws`/`gcloud`/`gh`/`ssh`. That protects against
other local users and offline disk theft (with full-disk encryption
on), but not against anything running as *your* user, root, or backup
tools that sweep `~/.config/`. Exclude
`~/.config/docket/credentials/` from backups and dotfile sync if that
matters to you, and prefer app passwords (which docket already uses)
over master passwords.

---

## Troubleshooting

- **`docket` not found after install** — make sure `$(PREFIX)/bin` is
  on your `$PATH`.
- **Feeds article images look chunky / pixelated** — your terminal
  doesn't speak iTerm2 / Kitty / Sixel; docket fell back to unicode
  half-blocks.
- **A layout cell shows nothing / logs "unknown widget kind in
  layout, skipping"** — the `[[layout.cells]]` entry references a
  widget kind that isn't compiled in (slim build) or doesn't exist.
- **Logs**: `~/.config/docket/docket.log` (`tail -f` it while
  debugging — stderr/stdout would corrupt the alt-screen display).
- **Reset to defaults**: move aside `~/.config/docket/` and re-run
  `docket --init`.

---

## Contributing

1. `make test` — should pass clean on `main` at all times.
2. `make build` for debug, `make` for release.
3. `cargo clippy --features widgets-all` for lints.
4. **Adding a widget** is purely additive: implement the `Widget`
   trait under `src/widgets/<name>/`, declare a `widget-<name>` Cargo
   feature, and append a `WidgetDescriptor` to
   `src/widgets/registry.rs`. No edits to `app.rs`/`main.rs` needed.
5. `AGENTS.md` carries the architecture overview.

Issues and PRs welcome.

---

## License

docket is licensed under **GNU GPL v3 or later** — see
[LICENSE](LICENSE) for the full text. In short: you're free to use,
modify, and redistribute docket, but any modified version you
distribute must also be GPL-licensed and must keep docket's
copyright notices intact. See [CONTRIBUTING.md](CONTRIBUTING.md) for
the contributor sign-off + relicensing grant.

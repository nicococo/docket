# docket — Terminal Dashboard

## What This Is

docket is a keyboard-driven terminal dashboard (Rust + Ratatui) that
displays calendar, a tabbed multi-source RSS reader (Feeds), a
vim-flavoured notes pad, and email in one **fixed** 4-pane layout —
docket is deliberately opinionated, not a configurable canvas (see
`src/pane_layout.rs`). A young fork of
[glint](https://github.com/ntrospect0/glint) (see `README.md` →
Origins); shipping under GPL v3-or-later (see `LICENSE` +
`CONTRIBUTING.md`).

For user-facing setup, see `README.md` and `INSTRUCTIONS.md`.

## Tech Stack

- **Rust 2021 edition** with Tokio async runtime
- **Ratatui 0.28+** for TUI rendering (crossterm backend)
- **reqwest** for HTTP (RSS feeds, CalDAV, ICS feeds, Anthropic +
  OpenAI APIs; IMAP itself goes over the `imap` crate, not HTTP). A
  single process-wide client lives in `src/http.rs::shared()`.
- **serde + toml** for config; **serde_json** for API responses
- **chrono + chrono-tz** for timezone-aware date/time
- **strsim** for fuzzy command matching
- **feed-rs** for RSS/Atom parsing (Feeds widget)
- **ratatui-image + image** for the Feeds widget's inline article images
- **imap + native-tls + mail-parser** for the Email widget's IMAP path
- **lru + sha2** for in-memory caches keyed by content hash
- Config format: **TOML** (not JSON, not YAML)

## Project Structure

```
src/
├── main.rs                  # Entry point, CLI parsing (clap), runtime setup
├── app.rs                   # App state, focus model, command dispatch, live-reload
├── event.rs                 # Event loop: crossterm input + tick + config watch
├── http.rs                  # Process-wide shared reqwest::Client (`shared()`)
├── clipboard.rs             # OSC-52 clipboard write helper
├── runtime_state.rs         # Per-process state persisted to ~/.config/docket/.runtime_state.toml
├── pane_layout.rs           # PaneAreas — the fixed 4-pane rects (calendar/feeds/notes/email)
├── cache/
│   └── mod.rs               # Persistent on-disk cache (JSON + bytes). See Cache Layer.
├── config/
│   ├── mod.rs               # Single config.toml loader; widget_section_json bridge; XDG paths; --init seed
│   ├── types.rs             # Top-level Config struct ([global] + one field per widget)
│   └── watcher.rs           # `notify`-based watcher on config.toml
├── widgets/
│   ├── mod.rs               # Widget trait, WidgetCtx, WidgetManager
│   ├── registry.rs          # WIDGETS table — add a descriptor to register
│   ├── calendar/, email/, notes/, feeds/
│   │   └── mod.rs + helpers — each is a self-contained widget module
│   │      with `pub const KIND` and `pub fn build(&WidgetCtx)`.
├── llm/
│   ├── mod.rs               # LlmProvider trait, LlmProviderDef registry,
│   │                        #   LlmRequest/LlmResponse types
│   ├── anthropic.rs         # AnthropicProvider (Messages API)
│   ├── openai.rs            # OpenAiProvider (Chat Completions API)
│   ├── rate_limiter.rs      # Token-bucket request budget tracking
│   └── cache.rs             # In-memory LRU response cache (L1), TTL-evicted
├── theme/
│   ├── mod.rs               # Color scheme loader, per-widget overrides
│   └── builtin_schemes.toml # Compiled-in palettes (not a user-editable file)
└── ui/
    ├── mod.rs               # Top-level renderer, unified title row helper,
    │                        #   command bar, status bar wiring
    ├── status_bar.rs        # Bottom bar: theme, command feedback, clock
    ├── help.rs              # "?" help overlay (sources keybindings from widgets)
    └── big_digits.rs        # Block-digit renderer used by Calendar's day view
```

## Core Traits and Registries

Five extension points define the architecture — know these before
touching anything material.

### Widget (src/widgets/mod.rs)
The runtime contract for each of the 4 fixed panes. The trait is
broader than this excerpt (mouse, keybindings, shortcut prefs, theme
reload) — see the source for the full surface. There is no more
"stack"/composite-child concept — each pane holds exactly one widget.

```rust
pub trait Widget: Send + Sync {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn kind(&self) -> &str;
    fn instance(&self) -> &str { "main" }
    async fn update(&mut self, ctx: &AppContext) -> Result<()>;
    fn render(&self, frame: &mut Frame, area: Rect, focused: bool);
    fn handle_key(&mut self, key: KeyEvent) -> EventResult;
    fn handle_mouse(&mut self, _: MouseEvent, _: Rect) -> EventResult { /* ignored */ }
    fn handle_command(&mut self, cmd: &str, args: &[&str]) -> Result<bool>;
    fn config(&self) -> serde_json::Value;
    fn apply_config(&mut self, config: serde_json::Value) -> Result<()>;
    fn keybindings(&self) -> Vec<(&'static str, &'static str)> { vec![] }
    fn set_app_theme(&mut self, _: Arc<Theme>) {}
    fn shortcut_preferences(&self) -> &[char] { &[] }
    fn set_shortcut(&mut self, _: Option<char>) {}
}
```

### WidgetCtx (src/widgets/mod.rs)
Construction-time bundle every widget factory receives:

```rust
pub struct WidgetCtx {
    pub instance: String,                       // "main", or "ai" for the feeds pane — cache-scoping only now
    pub theme: Arc<Theme>,
    pub llm: Option<Arc<dyn LlmProvider>>,      // None when LLM disabled / unconfigured
    pub cache: ScopedCache,                     // already scoped to (kind, instance)
    pub config: serde_json::Value,              // this widget's table from config.toml, e.g. `[calendar]`
}
```

### LlmProvider + LlmProviderDef registry (src/llm/mod.rs)
The provider trait is the runtime boundary; `LlmProviderDef` is the
registry entry the config layer uses to dispatch builds correctly.

```rust
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse>;
}

pub struct LlmProviderDef {
    pub name: &'static str,                    // matched against llm.toml [provider].name
    pub display_name: &'static str,
    pub credentials_filename: &'static str,    // under credentials/
    pub key_portal_url: &'static str,
    pub default_model: &'static str,
    pub default_api_base: &'static str,
    pub default_max_tokens: u32,
    pub builder: LlmBuilder,                   // fn(&ProviderConfig, LimitsConfig) -> Result<Option<Arc<dyn LlmProvider>>>
}

pub const PROVIDERS: &[LlmProviderDef] = &[ /* anthropic, openai */ ];
```

Add a new LLM provider by appending one entry to `PROVIDERS`.

### WidgetDescriptor + registry (src/widgets/registry.rs)
The single registration point for widget kinds:

```rust
pub struct WidgetDescriptor {
    pub kind: &'static str,
    pub factory: WidgetFactory,                // &WidgetCtx -> Box<dyn Widget>
}

pub const WIDGETS: &[WidgetDescriptor] = &[ /* calendar, email, notes, feeds */ ];
```

## Adding a New Widget

docket is deliberately a **fixed 4-pane app**, not an extensible
widget canvas — adding a 5th pane means also deciding where it lives
in `src/pane_layout.rs` and `app.rs`'s `register_default_widgets`,
which is a real design decision, not a drop-in extension point like
it used to be. If you're doing this anyway:

1. Create `src/widgets/<name>/mod.rs` implementing the `Widget` trait.
2. Export at module level: `pub const KIND: &str = "<name>";` and
   `pub fn build(ctx: &WidgetCtx) -> Box<dyn Widget>`.
3. Add a `widget-<name>` feature in `Cargo.toml` and a
   `#[cfg(feature = "widget-<name>")] pub mod <name>;` line in
   `src/widgets/mod.rs`. Add to the `widgets-all` umbrella.
4. Append a `WidgetDescriptor` to `WIDGETS` in
   `src/widgets/registry.rs`.
5. Add it to `PaneAreas`/`FOCUS_ORDER` in `src/pane_layout.rs` and to
   `register_default_widgets` in `src/app.rs` — this is the part that
   used to be config-driven and now isn't.
6. Add its config struct as a field on `Config` in `src/config/types.rs`
   and to the merged seed at `src/config/defaults/config.toml`.

The cache scope is granted automatically by the factory contract; a
fresh instance's config is whatever `T::default()` produces if its
table is absent from `config.toml`.

If your widget fetches remote data, use `ctx.cache` (see **Cache
Layer** below) and `crate::http::shared()` for the HTTP client unless
you need bespoke session state (CalDAV's basic-auth headers — those
construct their own `reqwest::Client`).

If it needs a credential (app password, API key), read it via
`crate::credentials::load(filename)` (see `src/credentials.rs`) — every
credential is a hand-edited TOML file under
`~/.config/docket/credentials/`; there's no OAuth flow in this codebase.

If it talks to an LLM, accept the optional `ctx.llm` and gate on a
per-widget TOML flag (`summarize_with_llm = true`).

## Config System

Everything lives in one file, `~/.config/docket/config.toml`:

```
[global]              — theme, mouse_scroll, background_poll_ratio, zoom_margin, …
[calendar]             — providers, calendar_ids, [[calendar.events]]
[feeds]                 — [[feeds.feeds]] (one tabbed multi-source RSS pane)
[email]                 — provider, [[email.accounts]], summarize_with_llm
[notes]                 — notes_dir
[llm]                   — [llm.provider] name (anthropic / openai), model, [llm.limits]
notes/<instance>/      — one .md per note; mtime sorts the list
credentials/            — API keys, IMAP/CalDAV passwords (0600) — the one thing NOT in config.toml
```

Colour palettes are compiled into the binary (`src/theme/builtin_schemes.toml`,
embedded via `include_str!` + `LazyLock`) — no on-disk colorschemes file.

`config::load_raw()` parses `config.toml` once into a raw `toml::Value`;
`config::widget_section_json(raw, "calendar")` pulls out one table as
`serde_json::Value` for `WidgetCtx.config` — this is the bridge that
lets widget config structs skip deriving `Serialize`. `Config` in
`src/config/types.rs` also carries typed fields (`calendar`, `feeds`,
`notes`, `email`, `llm`) for direct in-memory reads (e.g. email's
extract-actions reading `config.notes.notes_dir`) and as what the
seed file is verified against in tests.

`config.toml` is watched via `notify` (parent-dir, non-recursive —
editors rename-on-save). On any change, `apply_config_change` reparses
the whole file and calls `apply_config` on all 4 registered panes
unconditionally (cheap, idempotent — no per-file dispatch needed
anymore since there's only one file). The user can also force reload
with `:reload`. **Known gap**: `[llm]` changes reparse into
`app.config.llm` but aren't pushed into already-constructed widgets'
`Arc<dyn LlmProvider>` — that needs a new `Widget` trait hook across
every LLM-consuming widget, restart still required for LLM provider
changes.

## LLM Integration

Two providers ship today (Anthropic + OpenAI). Add more via the
`LlmProviderDef` registry. The active provider is set by
`llm.toml`'s `[provider].name`; widgets call through the trait without
knowing which concrete provider is active.

Follow the **structured-first, LLM-fallback** principle:

| Input type | Fast path (always try first) | LLM path (only if needed) |
|---|---|---|
| News relevance | Keyword substring match | (Reserved for batch classification) |
| News article body | RSS `<description>` field | Page-extract via `readability` then summarise |
| Email summary | First N chars of plain_body | On-demand `s`-key summary |
| Date parsing | chrono / chrono-english | Not needed |
| Typo correction | strsim edit-distance | Not needed |

Every LLM feature MUST have a non-LLM fallback. No widget should break
if the API key is missing or the service is down.

LLM responses are cached in two tiers: an in-memory LRU (128 entries,
7-day TTL) in `src/llm/cache.rs`, and a per-widget on-disk cache keyed
by `sha256(record-id)` for content-stable derivations.

## Cache Layer

Widgets that fetch remote data should persist results so the dashboard
paints prior values on the first frame and refreshes in the background.
`src/cache/mod.rs` provides two parallel APIs — JSON for structured
payloads and bytes for opaque blobs:

```rust
// JSON values (any T: Serialize + DeserializeOwned)
ctx.cache.load::<T>(key) -> Option<CacheEntry<T>>
ctx.cache.store(key, &value) -> Result<()>

// Raw bytes (images, attachments)
ctx.cache.load_bytes(key) -> Option<BytesEntry>
ctx.cache.store_bytes(key, &bytes) -> Result<()>

// Maintenance
ctx.cache.invalidate(key) -> Result<()>   // clears both .json and .bin variants
```

`ctx.cache` is already scoped to the widget's `(kind, instance)`. Pick
any flat key namespace per widget (`articles`, `quotes-1d`, `messages`,
`thumb-<hash>`). Files land at
`~/.cache/docket/<kind>/<instance>/<key>.{json,bin}`. Writes are atomic
(temp + rename); a crash mid-write can't corrupt an existing entry.

On startup the app runs `Cache::sweep_older_than(30d)` to drop orphan
cache files left by removed widgets or renamed instances.

### Fetch-payload pattern (reference: `feeds/mod.rs`)

1. In the constructor, `cache.load::<T>(key)` and seed the in-memory state.
2. Translate `entry.age()` into a synthetic `Instant` so the existing
   `last_attempt` poll-interval gate suppresses redundant refetches:
   `last_attempt = Some(Instant::now() - entry.age().min(poll_interval))`.
3. After a successful fetch, `cache.store(key, &payload)`. Failures log
   and continue — caching is best-effort.

### LLM-derivation pattern (reference: `feeds/mod.rs`, `email/mod.rs`)

Cache per-record derivations (article summaries, message summaries)
when they're content-stable. Key by a SHA-256 prefix of the record's
identity (article URL, message ID). Only persist successful outcomes —
flaky network calls shouldn't poison the cache.

### Bytes-payload pattern (reference: `feeds/image.rs`)

For files that take longer to decode than to read (resized article
images), cache the heavy output keyed by source-path hash and
invalidate against the source file's mtime.

## Memory + HTTP Discipline

Two cross-cutting concerns the platform takes care of so individual
widgets don't reinvent them.

### Shared HTTP client (src/http.rs)

`crate::http::shared()` returns a process-wide `reqwest::Client` (docket
UA, 30s timeout, no cookies). Callers include feeds, calendar (CalDAV,
ICS), LLM providers. Per-request timeout overrides via
`RequestBuilder::timeout` where a shorter bound matters.

Bespoke clients deliberately kept for callers needing client-scoped state:
- **calendar/caldav**: per-request Basic auth header pushed via
  `default_headers` — would leak onto unrelated requests if shared.

### Bounded in-memory state

Every widget that accumulates data has a defined upper bound:
- Feeds: drops summaries for articles that rotated out of the feed.
- Email: caps in-memory body length at 4 KB (full message read via `o`
  opens the user's mail client).
- LLM cache: 128 entries with 7-day TTL.

When you add a new widget, do the same — don't ship an unbounded
collector.

## Key Architectural Decisions

- **Graph rendering**: braille characters (U+2800–U+28FF) for the
  intraday + multi-period traces. Box-drawing fallback available.
- **Colours**: ANSI semantic colours (Red, Green, etc.) for default
  text inherit from the terminal theme. Each compiled-in colour scheme
  (`src/theme/builtin_schemes.toml`) overrides those defaults;
  per-pane `[colors]` blocks in `config.toml` further override per
  pane.
- **Cache**: persistent JSON + bytes under `~/.cache/docket/`; widgets
  seed on construction, refresh in background, persist on success.
- **Command routing**: focused widget gets priority; `:cmd` falls
  through to widgets in registration order. `widget_id:command` for
  explicit targeting.
- **Auth storage**: plain files with 0600 perms in `credentials/`
  (mirroring `gcloud`, `gh`, etc.). A post-v0.2 refactor moves this
  behind a `CredentialBackend` trait with three tiers — OS keychain
  (`keyring` crate), host-bound AES-GCM (key from `machine-uid`), and
  the current plaintext file as fallback — selected via
  `credentials_backend` in `config.toml`. See `CHANGELOG.md` →
  Deferred for the work breakdown. Pick:
  - **Keychain**: default on macOS, Windows, Linux desktop with a
    running Secret Service daemon.
  - **Host-bound**: default on Linux headless / SSH-only / homelab
    where Secret Service is unavailable. Leak-resistant (file is
    useless if copied off-host) but does NOT defend against local
    processes that can read the machine-ID.
  - **Plaintext**: universal fallback. Add Windows ACL handling
    before relying on it as the documented default there — `chmod
    0600` is a no-op on NTFS.
- **Calendar**: merged multi-calendar timeline with per-calendar colour
  coding.
- **Notes**: one `.md` file per note under
  `~/.config/docket/notes/<instance>/`; user can hand-edit, back up via
  git, or move between machines.

## Commands

```sh
cargo build --features widgets-all          # debug build, all widgets
cargo run                                    # debug binary with default config
cargo run -- --init                          # seed ~/.config/docket/
cargo test --features widgets-all            # full suite (~450 tests)
cargo clippy --features widgets-all          # lint
cargo fmt                                    # format
make install PREFIX=~/.local                 # build + copy to ~/.local/bin
```

## Conventions

- Use `Result<T>` with `anyhow` for error handling (no custom error
  types in v0.x).
- All async code runs on Tokio; never block the event loop. Use
  `tokio::task::spawn_blocking` for sync APIs like the `imap` crate.
- Widget rendering is synchronous (Ratatui requirement) — data
  fetching is async and writes results into shared state.
- Prefer `tracing` over `println!` / `eprintln!` — alt-screen mode
  would corrupt the TUI; tracing writes to `~/.config/docket/docket.log`.
- TOML config structs derive `serde::Deserialize` with `#[serde(default)]`
  for optional fields.
- Test data providers with mock HTTP responses (`wiremock` is in
  dev-deps).
- Every `.rs` file under `src/` carries an SPDX header
  (`// SPDX-License-Identifier: GPL-3.0-or-later`). The script in the
  prelaunch commit history is idempotent — re-run after adding a new
  file if your editor doesn't auto-insert.
- Comments default to **none**. Add one only when the WHY is non-obvious
  (hidden constraint, subtle invariant, workaround). Don't restate
  what well-named code already says.

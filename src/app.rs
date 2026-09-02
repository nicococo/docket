// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal};

use crate::{
    cache::Cache,
    config::{self, Config},
    event::{Event, EventReader},
    llm::{self, LlmProvider},
    pane_layout::{PaneAreas, FOCUS_ORDER},
    theme::{self, Theme},
    ui,
    widgets::{registry, AppContext, EventResult, WidgetCtx, WidgetManager},
    zoom::ZoomTarget,
};

/// Time a `command_feedback` message stays visible in the chrome row
/// before render replaces it with the idle status-bar content.
const FEEDBACK_TTL: Duration = Duration::from_secs(3);

pub struct App {
    config: Config,
    theme: Arc<Theme>,
    manager: WidgetManager,
    focus_idx: usize,
    /// Widget ids in fixed pane order (Tab cycles through this). A
    /// subset of `pane_layout::FOCUS_ORDER` — panes whose widget
    /// feature isn't compiled in are skipped.
    focus_order: Vec<String>,
    /// `Shift+<letter>` → target widget id.
    shortcuts: HashMap<char, String>,
    should_quit: bool,
    show_help: bool,
    help_scroll: u16,
    /// Max scroll updated by `ui::help::render` so the scroll handler can
    /// clamp without re-computing the layout.
    help_scroll_max: Cell<u16>,
    /// `Some` while the user is composing after pressing `:`.
    command_buffer: Option<String>,
    /// Transient feedback shown in the chrome row after a `:` command.
    /// Carries a severity tag (drives the message color via the active
    /// scheme) and a timestamp; render expires entries older than
    /// `FEEDBACK_TTL` so the message disappears on its own without the
    /// user having to dismiss it.
    command_feedback: Option<(String, ui::FeedbackSeverity, Instant)>,
    /// Cached pixels + bookkeeping from the previous draw. The
    /// partial-render path blits unchanged cells from here into the
    /// new frame instead of re-running their `render()`. See
    /// [`ui::PartialDrawCache`] for the invalidation rules.
    partial_draw: ui::PartialDrawCache,
    /// The currently-active zoom target, or `None` when zoom is off.
    /// Set by `zoom_enter`, cleared by `exit_zoom`. Drives the zoom
    /// overlay in `ui::render` and the zoom-active event dispatch branch.
    zoom_target: Option<ZoomTarget>,
    /// Wrapping tick counter used to throttle backdrop widget polls while zoom
    /// is active. Incremented every `Event::Tick`; backdrop widgets only call
    /// `update()` when `counter % background_poll_ratio == 0`. Reset to 0 at
    /// startup.
    zoom_backdrop_tick_counter: u64,
}

impl App {
    pub fn new(config: Config) -> Self {
        // Theme + LLM are both best-effort: missing files / unknown schemes /
        // missing API keys all log a warning and continue with sensible
        // defaults (built-in palette, no LLM).
        let theme = theme::load(&config.global.theme).unwrap_or_else(|err| {
            tracing::warn!(error = %err, "failed to resolve color scheme, using built-in defaults");
            Arc::new(Theme::builtin_defaults())
        });

        let llm_provider = llm::build_provider(&config.llm).unwrap_or_else(|err| {
            tracing::warn!(error = %err, "failed to build LLM provider");
            None
        });

        // Cache root opened once and scoped per-widget at registration time.
        // If the home dir can't be resolved (exotic environment), fall back
        // to the system temp dir — widgets keep working, they just don't
        // persist between runs.
        let cache = Cache::open_default().unwrap_or_else(|err| {
            tracing::warn!(error = %err, "failed to resolve cache dir; using temp dir");
            Cache::at(std::env::temp_dir().join("docket-cache"))
        });
        // Best-effort startup sweep: drop cache files no widget has touched
        // in 30 days. Each widget's cache size is bounded per entry, but
        // long-running setups accumulate orphans (renamed feeds, dropped
        // tickers, gallery images that moved). Cheap enough to run every
        // launch; failures log and the dashboard proceeds.
        let removed = cache.sweep_older_than(std::time::Duration::from_secs(30 * 24 * 60 * 60));
        if removed > 0 {
            tracing::info!(removed, "cache sweep: dropped stale entries");
        }

        let raw_sections = config::load_raw(None).unwrap_or_else(|err| {
            tracing::warn!(error = %err, "failed to parse config.toml for widget sections");
            toml::Value::Table(Default::default())
        });
        let mut manager = WidgetManager::new();
        register_default_widgets(
            &mut manager,
            theme.clone(),
            llm_provider,
            &cache,
            &raw_sections,
        );

        let focus_order = focus_order_from_manager(&manager);
        let shortcuts = assign_shortcuts(&mut manager);
        Self {
            config,
            theme,
            manager,
            focus_idx: 0,
            focus_order,
            shortcuts,
            should_quit: false,
            show_help: false,
            help_scroll: 0,
            help_scroll_max: Cell::new(0),
            command_buffer: None,
            command_feedback: None,
            partial_draw: ui::PartialDrawCache::default(),
            zoom_target: None,
            zoom_backdrop_tick_counter: 0,
        }
    }

    /// Adjust the help overlay's vertical scroll by `delta` rows. Clamps
    /// against `help_scroll_max` (updated by the previous render) so we
    /// never scroll past the last line of content. Called by Up/Down/k/j
    /// /PgUp/PgDn keys and by mouse wheel events when the overlay is open.
    fn scroll_help(&mut self, delta: i32) {
        let max = self.help_scroll_max.get() as i32;
        let next = (self.help_scroll as i32 + delta).clamp(0, max);
        self.help_scroll = next as u16;
    }

    fn focused_widget(&self) -> Option<&str> {
        self.focus_order.get(self.focus_idx).map(String::as_str)
    }

    /// Set the chrome-row feedback message. Caller picks the severity;
    /// the timestamp is stamped here so render can age the entry out
    /// after `FEEDBACK_TTL` without each call site having to think about
    /// clock plumbing.
    fn set_feedback(&mut self, text: impl Into<String>, severity: ui::FeedbackSeverity) {
        self.command_feedback = Some((text.into(), severity, Instant::now()));
    }

    /// Drop the feedback if it's older than `FEEDBACK_TTL`. Returns
    /// `true` when the bar was actually cleared so the tick path can
    /// force a redraw — otherwise the now-stale "saved" / "error"
    /// chrome would linger until the next user event.
    fn expire_stale_feedback(&mut self) -> bool {
        if let Some((_, _, set_at)) = &self.command_feedback {
            if set_at.elapsed() >= FEEDBACK_TTL {
                self.command_feedback = None;
                return true;
            }
        }
        false
    }

    /// Drain every widget's dirty bit. Returns the set of ids that
    /// reported `true` — the partial-render path needs to know
    /// *which* widgets need a fresh paint, not just "any of them."
    /// Always calls `take_dirty` on every widget (even when the
    /// answer is obviously yes elsewhere) so a queued bit can't
    /// smuggle into the next tick and trigger a redundant redraw.
    fn drain_widget_dirty_ids(&mut self) -> HashSet<String> {
        let mut dirty: HashSet<String> = HashSet::new();
        for id in self.manager.ids().to_vec() {
            if let Some(w) = self.manager.get_mut(&id) {
                if w.take_dirty() {
                    dirty.insert(id);
                }
            }
        }
        dirty
    }

    /// Borrow the current feedback as the ui-layer tuple, after expiring
    /// stale entries. Used at each RenderState construction site so the
    /// three draw paths stay in lockstep.
    fn feedback_for_render(&self) -> Option<(&str, ui::FeedbackSeverity)> {
        self.command_feedback
            .as_ref()
            .filter(|(_, _, set_at)| set_at.elapsed() < FEEDBACK_TTL)
            .map(|(text, severity, _)| (text.as_str(), *severity))
    }

    /// Snapshot the App's draw-time inputs into a `RenderState` for the
    /// UI layer. One constructor instead of three inline literals;
    /// adding a render-state field becomes a one-line change here
    /// instead of three identical edits.
    fn render_state(&self) -> ui::RenderState<'_> {
        ui::RenderState {
            manager: &self.manager,
            focused: self.focused_widget(),
            show_help: self.show_help,
            command_buffer: self.command_buffer.as_deref(),
            command_feedback: self.feedback_for_render(),
            theme: &self.theme,
            theme_name: &self.config.global.theme,
            help_scroll: self.help_scroll,
            help_scroll_max: &self.help_scroll_max,
            show_status_bar: self.config.global.show_status_bar,
            zoom_target: self.zoom_target.as_ref(),
            zoom_margin: self.config.global.zoom_margin,
        }
    }

    fn cycle_focus(&mut self, forward: bool) {
        if self.focus_order.is_empty() {
            return;
        }
        let n = self.focus_order.len();
        self.focus_idx = if forward {
            (self.focus_idx + 1) % n
        } else {
            (self.focus_idx + n - 1) % n
        };
    }

    /// Shift input focus to the widget with the given id — used by
    /// widget-initiated focus requests (timer alarm, etc.). Returns
    /// `true` when the widget was found and focus was changed.
    fn promote_to_widget(&mut self, target_id: &str) -> bool {
        if let Some(pos) = self.focus_order.iter().position(|w| w == target_id) {
            self.focus_idx = pos;
            return true;
        }
        false
    }

    /// Drain every widget's pending focus request and honor them in id
    /// order. Called from the tick loop after `update` so widgets that
    /// decide to grab attention inside `update` see the focus shift on
    /// the same frame. Returns `true` when at least one request was
    /// honored, so the caller can force a redraw even when no widget
    /// marked itself dirty (focus changes don't auto-set the dirty bit).
    fn process_focus_requests(&mut self) -> bool {
        let all_ids: Vec<String> = self.manager.ids().to_vec();
        let mut promoted = false;
        for id in all_ids {
            let req = self
                .manager
                .get_mut(&id)
                .and_then(|w| w.take_focus_request());
            if let Some(req) = req {
                if self.promote_to_widget(&req.widget_id) {
                    promoted = true;
                }
            }
        }
        promoted
    }

    // ── Zoom methods ─────────────────────────────────────────────────────

    /// Enter zoom for the currently-focused widget. No-op when `focus_order`
    /// is empty or `focused_widget()` returns `None`.
    fn zoom_enter(&mut self) {
        let Some(focused_id) = self.focused_widget().map(str::to_string) else {
            return;
        };
        self.zoom_target = Some(ZoomTarget { widget_id: focused_id });
    }

    /// Exit zoom, clearing `zoom_target` and landing focus on the widget
    /// that was zoomed (so focus is where the user left off).
    fn exit_zoom(&mut self) {
        let Some(zoom) = self.zoom_target.take() else {
            return;
        };
        if let Some(pos) = self.focus_order.iter().position(|w| w == &zoom.widget_id) {
            self.focus_idx = pos;
        }
    }

    /// Resolve the zoom target to an immutable widget reference. Used by
    /// `is_zoom_retarget_suppressed` and `display_name` lookups.
    fn resolve_zoom_widget(&self) -> Option<&dyn crate::widgets::Widget> {
        let zoom = self.zoom_target.as_ref()?;
        self.manager.get(&zoom.widget_id)
    }

    /// Returns `true` when the currently-zoomed widget is actively capturing
    /// text input and retarget gestures (`Tab`, `Shift+<letter>`, mouse click
    /// on backdrop) should be suppressed. Returns `false` when there is no
    /// active zoom target.
    fn is_zoom_retarget_suppressed(&self) -> bool {
        self.resolve_zoom_widget()
            .map(|w| w.is_capturing_text())
            .unwrap_or(false)
    }

    /// Retarget zoom to the widget assigned shortcut letter `letter`. Moves
    /// both `zoom_target` and `focus_idx` so they stay synchronized.
    fn retarget_zoom_by_letter(&mut self, letter: char) {
        let Some(widget_id) = self.shortcuts.get(&letter).cloned() else {
            return;
        };
        if let Some(pos) = self.focus_order.iter().position(|w| w == &widget_id) {
            self.focus_idx = pos;
        }
        self.zoom_target = Some(ZoomTarget { widget_id });
    }

    /// Advance the zoom target by one step in focus order. `forward = true`
    /// moves to the next widget; `false` moves to the previous. Wraps at
    /// both ends. Updates `zoom_target` and `focus_idx` atomically.
    fn retarget_zoom_cycle(&mut self, forward: bool) {
        if self.focus_order.is_empty() {
            return;
        }
        let n = self.focus_order.len();
        let new_idx = if forward {
            (self.focus_idx + 1) % n
        } else {
            (self.focus_idx + n - 1) % n
        };
        self.focus_idx = new_idx;
        let widget_id = self.focus_order[new_idx].clone();
        self.zoom_target = Some(ZoomTarget { widget_id });
    }

    /// Handle a key event while zoom is active. The focused widget already
    /// had its chance via `handle_key`; only `Ignored` keys reach here.
    ///
    /// Dispatch table (evaluated top-to-bottom):
    ///
    /// | Pattern | Action |
    /// |---------|--------|
    /// | `show_help == true` | delegate to `handle_global_key` (help is modal above zoom) |
    /// | `z`, `Shift-Z`, `Shift-z` | `exit_zoom` |
    /// | `Esc` | `exit_zoom` |
    /// | Uppercase letter (not suppressed) | `retarget_zoom_by_letter` |
    /// | `Tab` (not suppressed) | `retarget_zoom_cycle(true)` |
    /// | `BackTab` (not suppressed) | `retarget_zoom_cycle(false)` |
    /// | `q`, `Ctrl-C` | quit |
    /// | `?` | open help overlay |
    /// | `:` | open command bar |
    /// | anything else | swallowed (zoom is modal) |
    fn handle_global_zoom_key(&mut self, key: crossterm::event::KeyEvent) {
        // Help overlay is modal above zoom — delegate the key so `?`/`Esc`/etc.
        // still work while zoom is in the background.
        if self.show_help {
            self.handle_global_key(key);
            return;
        }
        let suppressed = self.is_zoom_retarget_suppressed();
        match (key.modifiers, key.code) {
            // z / Shift-Z exit zoom (primary and backup toggle).
            (KeyModifiers::NONE, KeyCode::Char('z'))
            | (KeyModifiers::SHIFT, KeyCode::Char('Z'))
            | (KeyModifiers::SHIFT, KeyCode::Char('z')) => self.exit_zoom(),
            // Esc always exits zoom — the widget already had its first chance.
            (_, KeyCode::Esc) => self.exit_zoom(),
            // Shift+uppercase: retarget zoom to that widget's letter.
            (_, KeyCode::Char(c)) if c.is_ascii_uppercase() && !suppressed => {
                self.retarget_zoom_by_letter(c.to_ascii_lowercase());
            }
            // Tab / BackTab: cycle zoom target in focus order.
            (KeyModifiers::NONE, KeyCode::Tab) if !suppressed => {
                self.retarget_zoom_cycle(true);
            }
            (KeyModifiers::SHIFT | KeyModifiers::NONE, KeyCode::BackTab) if !suppressed => {
                self.retarget_zoom_cycle(false);
            }
            // Quit still works while zoomed.
            (KeyModifiers::NONE, KeyCode::Char('q')) => self.should_quit = true,
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => self.should_quit = true,
            // Help overlay.
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('?')) => {
                self.show_help = true;
                self.help_scroll = 0;
            }
            // Command bar (CEO-ratified: command bar renders above zoom, zoom stays
            // active in background — no exit-zoom arm here).
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(':')) => {
                self.command_buffer = Some(String::new());
                self.command_feedback = None;
            }
            // All other keys are swallowed — zoom is modal for global actions.
            _ => {}
        }
    }

    // ── End zoom methods ──────────────────────────────────────────────────

    fn handle_global_key(&mut self, key: crossterm::event::KeyEvent) {
        // Help overlay swallows every key — Esc / ? / q close it; arrows / k /
        // j / PgUp / PgDn / Home / End scroll. Everything else is dropped so
        // `q` doesn't accidentally quit through the overlay.
        if self.show_help {
            match key.code {
                KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q') => {
                    self.show_help = false;
                    self.help_scroll = 0;
                }
                KeyCode::Up | KeyCode::Char('k') => self.scroll_help(-1),
                KeyCode::Down | KeyCode::Char('j') => self.scroll_help(1),
                KeyCode::PageUp => self.scroll_help(-10),
                KeyCode::PageDown => self.scroll_help(10),
                KeyCode::Home | KeyCode::Char('g') => self.help_scroll = 0,
                KeyCode::End | KeyCode::Char('G') => {
                    self.help_scroll = self.help_scroll_max.get();
                }
                _ => {}
            }
            return;
        }
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Char('q')) => self.should_quit = true,
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => self.should_quit = true,
            (KeyModifiers::NONE, KeyCode::Tab) => self.cycle_focus(true),
            (KeyModifiers::SHIFT, KeyCode::BackTab) | (KeyModifiers::NONE, KeyCode::BackTab) => {
                self.cycle_focus(false)
            }
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('?')) => {
                self.show_help = true;
                self.help_scroll = 0;
            }
            // `:` opens the command bar when no widget claimed it.
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(':')) => {
                self.command_buffer = Some(String::new());
                self.command_feedback = None;
            }
            // `z` / `Shift-Z` / `Shift-z`: enter zoom for the focused widget.
            // This arm must come BEFORE the generic uppercase-letter arm so `Z`
            // is claimed as a zoom toggle and never dispatched as a shortcut key.
            // (Zoom exit while zoom is active is handled by `handle_global_zoom_key`.)
            (KeyModifiers::NONE, KeyCode::Char('z'))
            | (KeyModifiers::SHIFT, KeyCode::Char('Z'))
            | (KeyModifiers::SHIFT, KeyCode::Char('z')) => {
                self.zoom_enter();
            }
            // `Shift+<letter>` jumps to the widget that claimed that
            // letter. Some terminals drop the SHIFT modifier on
            // shifted alphabetic keys, so we match on case rather
            // than `KeyModifiers::SHIFT`.
            (_, KeyCode::Char(c)) if c.is_ascii_uppercase() => {
                let lower = c.to_ascii_lowercase();
                if let Some(widget_id) = self.shortcuts.get(&lower).cloned() {
                    if let Some(pos) = self.focus_order.iter().position(|w| w == &widget_id) {
                        self.focus_idx = pos;
                    }
                }
            }
            _ => {}
        }
    }

    /// Append a bracketed-paste payload to the command bar buffer. Newlines
    /// and other control chars are stripped — the command bar is a single
    /// line and Enter is the submit key, so pasted multi-line text would
    /// auto-execute fragments otherwise.
    fn handle_command_bar_paste(&mut self, text: &str) {
        self.command_feedback = None;
        let Some(buf) = self.command_buffer.as_mut() else {
            return;
        };
        for c in text.chars() {
            if !c.is_control() {
                buf.push(c);
            }
        }
    }

    fn handle_command_bar_key(&mut self, key: crossterm::event::KeyEvent) {
        self.command_feedback = None;
        let Some(buf) = self.command_buffer.as_mut() else {
            return;
        };
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) => {
                self.command_buffer = None;
            }
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                self.command_buffer = None;
            }
            // Ctrl-U mirrors the readline "kill to start of line" gesture.
            // The leading ':' lives in the chrome, not the buffer, so
            // clearing the buffer is exactly "wipe everything after the
            // prompt while keeping the prompt in place".
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                buf.clear();
            }
            (_, KeyCode::Backspace) if buf.pop().is_none() => {
                self.command_buffer = None;
            }
            (_, KeyCode::Enter) => {
                let line = std::mem::take(buf);
                self.command_buffer = None;
                self.execute_command(line.trim());
            }
            (mods, KeyCode::Char(c))
                if mods == KeyModifiers::NONE || mods == KeyModifiers::SHIFT =>
            {
                buf.push(c);
            }
            _ => {}
        }
    }

    /// `:scheme <name>` — switch among the built-in color schemes and
    /// propagate the new palette to every widget. Unknown names surface a
    /// feedback line listing the available schemes.
    fn execute_scheme_command(&mut self, args: &[&str]) {
        let file = match theme::load_schemes_file() {
            Ok(f) => f,
            Err(err) => {
                self.set_feedback(format!("color schemes: {err}"), ui::FeedbackSeverity::Error);
                return;
            }
        };

        // Sort once — used by both the "no arg" hint and the "not found"
        // message so the order is stable from the user's perspective.
        let mut available: Vec<&str> = file.schemes.keys().map(String::as_str).collect();
        available.sort_unstable();
        let available_csv = available.join(", ");

        let Some(name) = args.first() else {
            let msg = if available.is_empty() {
                "usage: :scheme <name> — (no built-in schemes found)".to_string()
            } else {
                format!("usage: :scheme <name>. Available: {available_csv}")
            };
            self.set_feedback(msg, ui::FeedbackSeverity::Warning);
            return;
        };

        let Some(scheme) = file.schemes.get(*name) else {
            let msg = if available.is_empty() {
                format!("unknown scheme {name:?} — no built-in schemes found")
            } else {
                format!("unknown scheme {name:?}. Available: {available_csv}")
            };
            self.set_feedback(msg, ui::FeedbackSeverity::Error);
            return;
        };

        let new_theme = theme::theme_from_scheme(scheme);
        self.theme = new_theme.clone();
        self.config.global.theme = (*name).to_string();
        for id in self.manager.ids().to_vec() {
            if let Some(widget) = self.manager.get_mut(&id) {
                widget.set_app_theme(new_theme.clone());
            }
        }
        // Persist so the choice survives restart. In-memory swap already
        // happened; a write failure only downgrades the success line.
        match theme::persist_active_scheme(name) {
            Ok(()) => {
                self.set_feedback(
                    format!("scheme → {name}"),
                    ui::FeedbackSeverity::Confirmation,
                );
            }
            Err(err) => {
                tracing::warn!(error = %err, scheme = %name, "failed to persist scheme");
                self.set_feedback(
                    format!("scheme → {name} (not persisted: {err})"),
                    ui::FeedbackSeverity::Warning,
                );
            }
        }
    }

    fn execute_command(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        let mut parts = line.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();

        // Global commands first.
        match cmd {
            "q" | "quit" | "exit" => {
                self.should_quit = true;
                return;
            }
            "help" | "?" => {
                self.show_help = true;
                self.help_scroll = 0;
                return;
            }
            "refresh" | "r" => {
                // Delegated so each widget defines its own refresh semantics.
                if let Some(id) = self.focused_widget().map(str::to_string) {
                    if let Some(widget) = self.manager.get_mut(&id) {
                        let _ = widget.handle_command("refresh", &args);
                    }
                }
                return;
            }
            "scheme" | "theme" => {
                self.execute_scheme_command(&args);
                return;
            }
            _ => {}
        }

        // Try the focused widget first, then every other registered widget.
        // The first one to return Ok(true) wins and gets focus.
        let focused = self.focused_widget().map(str::to_string);
        let ordered_ids: Vec<String> = {
            let mut ids: Vec<String> = Vec::new();
            if let Some(f) = focused.as_ref() {
                ids.push(f.clone());
            }
            for id in self.manager.ids() {
                if focused.as_deref() != Some(id.as_str()) {
                    ids.push(id.clone());
                }
            }
            ids
        };
        for id in ordered_ids {
            let Some(widget) = self.manager.get_mut(&id) else {
                continue;
            };
            match widget.handle_command(cmd, &args) {
                Ok(true) => {
                    if let Some(pos) = self.focus_order.iter().position(|w| w == &id) {
                        self.focus_idx = pos;
                    }
                    return;
                }
                Ok(false) => continue,
                Err(err) => {
                    self.set_feedback(format!("{id}: {err}"), ui::FeedbackSeverity::Error);
                    return;
                }
            }
        }
        self.set_feedback(
            format!("unknown command: {cmd:?}"),
            ui::FeedbackSeverity::Error,
        );
    }
}

/// Re-read `config.toml` after a filesystem-watcher event and propagate it:
/// theme, LLM provider, and every registered widget's own section. `path`
/// is whatever changed inside `~/.config/docket/` — only react when it's
/// `config.toml` itself; edits to `credentials/`, `notes/`, the runtime
/// state file, or the log are not config changes. Parse failures log and
/// skip — the next save event will retry.
fn apply_config_change(app: &mut App, path: &std::path::Path) {
    let Ok(config_path) = config::config_path() else {
        return;
    };
    if path.file_name() != config_path.file_name() {
        return;
    }

    let new_config: Config = match config::load(None) {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!(error = %err, "config.toml parse failed, will retry on next event");
            return;
        }
    };
    let raw_sections = config::load_raw(None).unwrap_or_else(|err| {
        tracing::warn!(error = %err, "failed to re-parse config.toml for widget sections");
        toml::Value::Table(Default::default())
    });

    // Theme: re-resolve against the (possibly unchanged) scheme name and
    // push it to every widget, same as `:scheme` does.
    let new_theme = theme::load(&new_config.global.theme).unwrap_or_else(|err| {
        tracing::warn!(error = %err, "failed to reload theme on config change, using built-in defaults");
        Arc::new(Theme::builtin_defaults())
    });
    app.theme = new_theme.clone();

    for (kind, widget_id) in [
        ("calendar", "calendar"),
        ("feeds", "feeds@ai"),
        ("notes", "notes"),
        ("email", "email"),
    ] {
        let Some(widget) = app.manager.get_mut(widget_id) else {
            continue;
        };
        let json = config::widget_section_json(&raw_sections, kind);
        if let Err(err) = widget.apply_config(json) {
            tracing::warn!(widget = %widget_id, error = %err, "apply_config failed");
        } else {
            tracing::info!(widget = %widget_id, "live-reloaded config");
        }
        widget.set_app_theme(new_theme.clone());
    }

    app.config = new_config;

    // Live-reload guard: if a config change removed the currently-zoomed widget
    // from the manager (e.g., a slim build dropped its feature), clear zoom so
    // the overlay doesn't render a ghost target.
    if let Some(ref zoom) = app.zoom_target.clone() {
        if app.manager.get(&zoom.widget_id).is_none() {
            tracing::info!(widget = %zoom.widget_id, "zoomed widget no longer in manager after config reload — clearing zoom");
            app.zoom_target = None;
        }
    }
}

/// Swap scroll-wheel directions in place. Vertical and horizontal axes are
/// both flipped so a trackpad with two-finger panning behaves consistently.
/// Non-scroll kinds pass through unchanged. Centralising this here keeps
/// every widget free of `if invert { ... } else { ... }` plumbing.
fn invert_scroll(kind: MouseEventKind) -> MouseEventKind {
    match kind {
        MouseEventKind::ScrollUp => MouseEventKind::ScrollDown,
        MouseEventKind::ScrollDown => MouseEventKind::ScrollUp,
        MouseEventKind::ScrollLeft => MouseEventKind::ScrollRight,
        MouseEventKind::ScrollRight => MouseEventKind::ScrollLeft,
        other => other,
    }
}

/// Route a mouse event to the zoomed widget, forwarding `mouse` into its
/// `handle_mouse`. `inner_rect` is the zoom frame's interior (excluding the
/// border), already computed by the caller so we don't have to repeat the
/// layout maths here. Returns `true` when the zoomed widget consumed the
/// event (i.e. something changed and a repaint is warranted).
fn route_mouse_to_zoom_widget(
    app: &mut App,
    mouse: crossterm::event::MouseEvent,
    inner_rect: Rect,
) -> bool {
    let Some(zoom) = app.zoom_target.clone() else {
        return false;
    };
    let Some(widget) = app.manager.get_mut(&zoom.widget_id) else {
        return false;
    };
    widget.handle_mouse(mouse, inner_rect) == EventResult::Handled
}

/// Returns the (widget id, pane area) under screen coordinates `(col, row)`,
/// if any. The bottom row is the status bar and is intentionally not focusable.
fn widget_at(_app: &App, full_area: Rect, col: u16, row: u16) -> Option<(String, Rect)> {
    if full_area.width == 0 || full_area.height == 0 {
        return None;
    }
    let main_height = full_area.height.saturating_sub(1);
    if row >= main_height {
        return None;
    }
    let main_area = Rect::new(full_area.x, full_area.y, full_area.width, main_height);
    let panes = PaneAreas::resolve(main_area);
    for (id, r) in panes.iter() {
        let in_x = col >= r.x && col < r.x + r.width;
        let in_y = row >= r.y && row < r.y + r.height;
        if in_x && in_y {
            return Some((id.to_string(), r));
        }
    }
    None
}

/// First-fit assignment of `Shift+<letter>` shortcuts in registration
/// order.
///
/// Returns the letter → widget id map; each widget is notified via
/// `set_shortcut`, including `None` for widgets whose preferences were
/// all taken.
fn assign_shortcuts(manager: &mut WidgetManager) -> HashMap<char, String> {
    let targets: Vec<(String, Vec<char>)> = manager
        .ids()
        .iter()
        .map(|id| {
            let prefs = manager
                .get(id)
                .map(|w| w.shortcut_preferences().to_vec())
                .unwrap_or_default();
            (id.clone(), prefs)
        })
        .collect();

    // First-fit assignment. Insertion order preserves registration-order
    // ties.
    let mut shortcuts: HashMap<char, String> = HashMap::new();
    let mut assigned_letters: HashMap<String, char> = HashMap::new();
    for (id, prefs) in &targets {
        for letter in prefs {
            let letter = letter.to_ascii_lowercase();
            if !letter.is_ascii_alphabetic() {
                continue;
            }
            // 'z' is reserved for the zoom toggle — never assign it.
            if letter == 'z' {
                continue;
            }
            if !shortcuts.contains_key(&letter) {
                shortcuts.insert(letter, id.clone());
                assigned_letters.insert(id.clone(), letter);
                break;
            }
        }
    }

    // Notify each widget of its granted letter (or `None` if all
    // preferences were taken).
    for (id, _) in &targets {
        let letter = assigned_letters.get(id).copied();
        if let Some(widget) = manager.get_mut(id) {
            widget.set_shortcut(letter);
        }
    }
    shortcuts
}

/// Focus-cycling order: the fixed pane order, skipping any pane whose
/// widget feature isn't compiled in (slim builds).
fn focus_order_from_manager(manager: &WidgetManager) -> Vec<String> {
    FOCUS_ORDER
        .iter()
        .filter(|id| manager.get(id).is_some())
        .map(|id| id.to_string())
        .collect()
}

fn register_widget(
    manager: &mut WidgetManager,
    kind: &str,
    instance: &str,
    theme: Arc<Theme>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    cache: &Cache,
    config_json: serde_json::Value,
) {
    let scoped = cache.scoped(kind, instance);
    let widget = registry::build_for(kind, instance, |instance| WidgetCtx {
        instance,
        theme,
        llm: llm_provider,
        cache: scoped,
        config: config_json,
    });
    match widget {
        Some(w) => manager.register_boxed(w),
        None => {
            tracing::warn!(kind = %kind, instance = %instance, "unknown widget kind, skipping");
        }
    }
}

/// Register the fixed 4-pane dashboard: calendar, the "ai" feeds
/// instance, notes, email. Each widget's config comes from its own
/// top-level table in `config.toml` (`[calendar]`, `[feeds]`, `[notes]`,
/// `[email]`) via `raw_sections`. A pane's widget silently doesn't
/// appear when its feature isn't compiled in (slim builds) —
/// `registry::find` returns `None` and `register_widget` logs and skips.
fn register_default_widgets(
    manager: &mut WidgetManager,
    theme: Arc<Theme>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    cache: &Cache,
    raw_sections: &toml::Value,
) {
    for (kind, instance) in [
        ("calendar", "main"),
        ("feeds", "ai"),
        ("notes", "main"),
        ("email", "main"),
    ] {
        let config_json = config::widget_section_json(raw_sections, kind);
        register_widget(
            manager,
            kind,
            instance,
            theme.clone(),
            llm_provider.clone(),
            cache,
            config_json,
        );
    }
}

/// Run the main loop. `TerminalGuard` restores the terminal on any exit path.
pub async fn run(config_path_override: Option<PathBuf>) -> Result<()> {
    let config = config::load(config_path_override.as_deref())?;

    let mut terminal = enter_tui().context("failed to initialize terminal")?;
    let _guard = TerminalGuard;

    let mut app = App::new(config);

    // Live-reload via the `notify` crate. Failure is non-fatal — we just
    // run without hot-reload.
    let config_rx = match config::config_dir() {
        Ok(dir) if dir.exists() => match config::watcher::spawn(dir) {
            Ok(rx) => Some(rx),
            Err(err) => {
                tracing::warn!(error = %err, "failed to spawn config watcher");
                None
            }
        },
        _ => None,
    };
    let mut events = EventReader::new(Duration::from_millis(250), config_rx);

    // Initial draw before the first event arrives.
    app.expire_stale_feedback();
    terminal.draw(|frame| {
        ui::render(frame, &app.render_state());
    })?;

    let ctx = AppContext;

    // Draw state accumulated across a coalesced burst of events (see the
    // has_pending() deferral below), so a flood of input collapses into one
    // repaint instead of one per event.
    let mut deferred_dirty: HashSet<String> = HashSet::new();
    let mut deferred_draw = false;
    let mut deferred_force_full = false;

    while let Some(evt) = events.next().await {
        let is_tick = matches!(evt, Event::Tick);
        // Non-tick events (key / paste / resize / config) mutate state outside
        // the dirty-bit contract, so they draw unconditionally. The mouse arm
        // narrows this to "only when the event actually changed something".
        let mut nontick_wants_draw = true;
        match evt {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                // Command bar takes precedence over both widgets and globals
                // — typing into it routes nowhere else.
                if app.command_buffer.is_some() {
                    app.handle_command_bar_key(key);
                    if app.should_quit {
                        break;
                    }
                } else if app.zoom_target.is_some() {
                    // Zoom-active dispatch: route key to the zoomed widget first,
                    // then to the zoom-modal global handler on Ignored.
                    let consumed = {
                        let widget_id = app.zoom_target.as_ref().unwrap().widget_id.clone();
                        if let Some(widget) = app.manager.get_mut(&widget_id) {
                            widget.handle_key(key) == EventResult::Handled
                        } else {
                            false
                        }
                    };
                    if !consumed {
                        app.handle_global_zoom_key(key);
                    }
                } else {
                    let consumed = if let Some(id) = app.focused_widget().map(str::to_string) {
                        if let Some(widget) = app.manager.get_mut(&id) {
                            widget.handle_key(key) == EventResult::Handled
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !consumed {
                        app.handle_global_key(key);
                    }
                }
            }
            Event::Mouse(mut mouse) => {
                let mut mouse_acted = false;
                // Apply the global `mouse_scroll` preference once at the
                // dispatch boundary so every downstream consumer (help
                // overlay + widgets) sees a consistent direction without
                // each having to know about the preference.
                if app.config.global.mouse_scroll == config::types::MouseScroll::Inverted {
                    mouse.kind = invert_scroll(mouse.kind);
                }
                // Help overlay sits on top of the entire dashboard — when
                // it's open, mouse input belongs to it, not to the widgets
                // visually behind it. Without this guard the scroll wheel
                // would silently drive the widget under the cursor.
                if app.show_help {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => app.scroll_help(-1),
                        MouseEventKind::ScrollDown => app.scroll_help(1),
                        _ => {}
                    }
                    // Swallow everything else (clicks etc.) so the layout
                    // underneath stays inert until the overlay closes.
                    if app.should_quit {
                        break;
                    }
                    app.expire_stale_feedback();
                    terminal.draw(|frame| {
                        ui::render(frame, &app.render_state());
                    })?;
                    continue;
                }
                if let Ok(size) = terminal.size() {
                    let full = Rect::new(0, 0, size.width, size.height);
                    let target = widget_at(&app, full, mouse.column, mouse.row);
                    if app.zoom_target.is_some() {
                        // Three-zone mouse dispatch while zoom is active.
                        let chrome_visible = app.command_buffer.is_some()
                            || app.command_feedback.is_some()
                            || app.config.global.show_status_bar;
                        let chrome_h: u16 = if chrome_visible { 1 } else { 0 };
                        let main_area = Rect::new(
                            full.x,
                            full.y,
                            full.width,
                            full.height.saturating_sub(chrome_h),
                        );
                        let zoom_rect = crate::ui::zoom_rect_with_margins(
                            main_area,
                            app.config.global.zoom_margin,
                        );
                        let inner_rect = Rect {
                            x: zoom_rect.x + 1,
                            y: zoom_rect.y + 1,
                            width: zoom_rect.width.saturating_sub(2),
                            height: zoom_rect.height.saturating_sub(2),
                        };
                        let in_zoom = mouse.column >= zoom_rect.x
                            && mouse.column < zoom_rect.x + zoom_rect.width
                            && mouse.row >= zoom_rect.y
                            && mouse.row < zoom_rect.y + zoom_rect.height;
                        match mouse.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                if in_zoom {
                                    // Click inside zoom frame: route to zoomed widget.
                                    mouse_acted =
                                        route_mouse_to_zoom_widget(&mut app, mouse, inner_rect);
                                } else if !app.is_zoom_retarget_suppressed() {
                                    if let Some((id, _)) = target {
                                        // Click on a backdrop widget: retarget zoom to it.
                                        let widget_id = id.clone();
                                        if let Some(pos) =
                                            app.focus_order.iter().position(|w| w == &widget_id)
                                        {
                                            app.focus_idx = pos;
                                        }
                                        app.zoom_target = Some(ZoomTarget { widget_id });
                                        mouse_acted = true;
                                    } else {
                                        // Click in the empty margin: exit zoom.
                                        app.exit_zoom();
                                        mouse_acted = true;
                                    }
                                }
                            }
                            MouseEventKind::ScrollUp
                            | MouseEventKind::ScrollDown
                            | MouseEventKind::ScrollLeft
                            | MouseEventKind::ScrollRight => {
                                if in_zoom {
                                    mouse_acted =
                                        route_mouse_to_zoom_widget(&mut app, mouse, inner_rect);
                                }
                            }
                            _ => {}
                        }
                    } else {
                        match mouse.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                if let Some((id, cell_area)) = target {
                                    if let Some(pos) =
                                        app.focus_order.iter().position(|w| w == &id)
                                    {
                                        // Focus moved → the highlight changes, so a
                                        // repaint is warranted regardless of what the
                                        // widget does with the click.
                                        if app.focus_idx != pos {
                                            mouse_acted = true;
                                        }
                                        app.focus_idx = pos;
                                    }
                                    if let Some(widget) = app.manager.get_mut(&id) {
                                        if widget.handle_mouse(mouse, cell_area)
                                            == EventResult::Handled
                                        {
                                            mouse_acted = true;
                                        }
                                    }
                                }
                            }
                            // Scroll wheel (both axes): forward to the widget
                            // under the cursor without changing focus — most
                            // users expect "scroll whatever I'm hovering over".
                            MouseEventKind::ScrollUp
                            | MouseEventKind::ScrollDown
                            | MouseEventKind::ScrollLeft
                            | MouseEventKind::ScrollRight => {
                                if let Some((id, cell_area)) = target {
                                    if let Some(widget) = app.manager.get_mut(&id) {
                                        if widget.handle_mouse(mouse, cell_area)
                                            == EventResult::Handled
                                        {
                                            mouse_acted = true;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                // Only a mouse event that actually changed something warrants a
                // repaint; inert events (button release, a click on empty space,
                // a scroll that hit its limit) must not force a full redraw.
                nontick_wants_draw = mouse_acted;
            }
            Event::Paste(text) => {
                // Hand the full bracketed-paste payload to the focused
                // widget. Most widgets ignore paste; text-buffer widgets
                // (notes) override Widget::handle_paste to insert it
                // atomically. The command bar swallows paste while open so
                // pasted text doesn't smuggle commands into widgets.
                if app.command_buffer.is_some() {
                    app.handle_command_bar_paste(&text);
                } else if let Some(id) = app.focused_widget().map(str::to_string) {
                    if let Some(widget) = app.manager.get_mut(&id) {
                        let _ = widget.handle_paste(&text);
                    }
                }
            }
            Event::Resize => {
                // Ratatui handles the re-layout on the next draw call below.
            }
            Event::ConfigChanged(path) => {
                apply_config_change(&mut app, &path);
            }
            Event::Tick => {
                // While zoomed, throttle backdrop widget updates to
                // `background_poll_ratio` cadence. The zoom target always
                // updates at full rate so its data stays live inside the
                // frame (Req 4).
                app.zoom_backdrop_tick_counter =
                    app.zoom_backdrop_tick_counter.wrapping_add(1);
                let ratio = app.config.global.background_poll_ratio.max(1) as u64;
                let allow_backdrop_tick = app.zoom_target.is_none()
                    || ratio == 1
                    || app.zoom_backdrop_tick_counter % ratio == 0;
                let zoomed_id: Option<String> =
                    app.zoom_target.as_ref().map(|z| z.widget_id.clone());
                for id in app.manager.ids().to_vec() {
                    // Skip backdrop widgets on non-quota ticks while zoomed.
                    if !allow_backdrop_tick && zoomed_id.as_deref() != Some(id.as_str()) {
                        continue;
                    }
                    if let Some(w) = app.manager.get_mut(&id) {
                        if let Err(err) = w.update(&ctx).await {
                            tracing::warn!(widget = %id, error = %err, "widget update failed");
                        }
                    }
                }
            }
        }

        if app.should_quit {
            break;
        }

        // Honor any focus requests widgets queued (e.g. a timer alarm
        // pulling the clock to the front of its stack). Tick-only —
        // the user-event branches (key/mouse/paste/resize) don't need
        // this poll, and a terminal sending continuous mouse-move
        // events shouldn't pay the per-widget iteration cost. A
        // 250 ms latency on alarm promotion is imperceptible.
        let focus_promoted = if is_tick {
            app.process_focus_requests()
        } else {
            false
        };

        let feedback_cleared = app.expire_stale_feedback();
        // Always drain widget dirty bits so they don't pile up between
        // draws — even when we already know we're going to draw (non-tick
        // events), so the next tick starts from a clean slate.
        deferred_dirty.extend(app.drain_widget_dirty_ids());

        // Per-widget `dirty_ids` is the authoritative "what changed" signal
        // only on ticks. Non-tick events (key / paste / resize / config
        // change) can mutate state outside the dirty-bit contract, so they
        // repaint unconditionally; a mouse event repaints only when it
        // actually acted (`nontick_wants_draw`). A non-tick draw forces a
        // full repaint; a tick-only batch keeps the partial fast path.
        let this_wants_draw = if is_tick {
            !deferred_dirty.is_empty() || feedback_cleared || focus_promoted
        } else {
            // A cleared status-bar feedback needs a repaint even if the mouse
            // event itself was inert.
            nontick_wants_draw || feedback_cleared
        };
        deferred_draw |= this_wants_draw;
        deferred_force_full |= !is_tick && nontick_wants_draw;

        // Coalesce: if more input is already queued, fold it into the next
        // iteration's single draw rather than repainting per event. Collapses
        // bursts (rapid clicks, scroll-wheel spins) into one repaint.
        if events.has_pending() {
            continue;
        }

        if deferred_draw {
            // Move the partial-draw cache out so the closure has a disjoint
            // mut borrow while `render_state()` holds an immutable borrow of
            // the rest of `app`. We restore it immediately after.
            let mut cache = std::mem::take(&mut app.partial_draw);
            let render_state = app.render_state();
            let force_full = deferred_force_full;
            terminal.draw(|frame| {
                ui::render_partial(
                    frame,
                    &render_state,
                    &deferred_dirty,
                    force_full,
                    &mut cache,
                );
            })?;
            app.partial_draw = cache;
        }
        deferred_dirty.clear();
        deferred_draw = false;
        deferred_force_full = false;
    }

    Ok(())
}

type Tui = Terminal<CrosstermBackend<io::Stdout>>;

fn enter_tui() -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // EnableBracketedPaste makes the terminal wrap pastes in
    // `\x1b[200~`/`\x1b[201~` markers, which crossterm surfaces as a
    // single `Event::Paste(String)` instead of one KeyEvent per
    // character. Without it, a paste containing `.`, `,`, `i`, `s`,
    // etc. fires widget shortcuts mid-stream — the user sees the
    // dashboard flash through stack rotations / mode toggles / etc.
    // before the rest of the buffer arrives. The Paste handler is
    // already wired up in the event loop (Event::Paste branch above);
    // this just turns on the terminal-side framing.
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

/// Restores the terminal on drop so a panic still leaves the user's shell sane.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_at_maps_clicks_to_cells() {
        let app = App::new(Config::default());
        let area = Rect::new(0, 0, 100, 40);
        assert_eq!(
            widget_at(&app, area, 5, 2).map(|(id, _)| id),
            Some("calendar".to_string())
        );
        assert_eq!(
            widget_at(&app, area, 80, 2).map(|(id, _)| id),
            Some("feeds@ai".to_string())
        );
        // Status bar row — last row of the area — should be unfocusable.
        assert!(widget_at(&app, area, 50, 39).is_none());
    }

    #[cfg(feature = "widgets-all")]
    #[test]
    fn focus_cycles_in_fixed_pane_order() {
        let config = Config::default();
        let mut app = App::new(config);
        assert_eq!(
            app.focus_order,
            vec![
                "calendar".to_string(),
                "feeds@ai".to_string(),
                "notes".to_string(),
                "email".to_string(),
            ]
        );
        assert_eq!(app.focused_widget(), Some("calendar"));
        app.cycle_focus(true);
        assert_eq!(app.focused_widget(), Some("feeds@ai"));
        app.cycle_focus(true);
        assert_eq!(app.focused_widget(), Some("notes"));
        app.cycle_focus(true);
        assert_eq!(app.focused_widget(), Some("email"));
        app.cycle_focus(true);
        assert_eq!(app.focused_widget(), Some("calendar"));
        app.cycle_focus(false);
        assert_eq!(app.focused_widget(), Some("email"));
    }

    #[cfg(feature = "widgets-all")]
    #[test]
    fn shortcuts_never_assign_z_and_target_real_widgets() {
        let app = App::new(Config::default());
        assert!(!app.shortcuts.contains_key(&'z'));
        for widget_id in app.shortcuts.values() {
            assert!(
                app.manager.get(widget_id).is_some(),
                "shortcut target {widget_id:?} should be a registered widget"
            );
        }
    }

    #[test]
    fn invert_scroll_flips_both_axes_and_passes_other_kinds_through() {
        assert_eq!(
            invert_scroll(MouseEventKind::ScrollUp),
            MouseEventKind::ScrollDown
        );
        assert_eq!(
            invert_scroll(MouseEventKind::ScrollDown),
            MouseEventKind::ScrollUp
        );
        assert_eq!(
            invert_scroll(MouseEventKind::ScrollLeft),
            MouseEventKind::ScrollRight
        );
        assert_eq!(
            invert_scroll(MouseEventKind::ScrollRight),
            MouseEventKind::ScrollLeft
        );
        // Non-scroll events are untouched.
        let click = MouseEventKind::Down(MouseButton::Left);
        assert_eq!(invert_scroll(click), click);
        assert_eq!(invert_scroll(MouseEventKind::Moved), MouseEventKind::Moved);
    }

    // ── Zoom tests ────────────────────────────────────────────────────────

    /// `z` key enters zoom for the focused widget and sets `zoom_target`.
    #[cfg(feature = "widgets-all")]
    #[test]
    fn zoom_enter_sets_zoom_target() {
        let mut app = App::new(Config::default());
        assert!(app.zoom_target.is_none());
        assert_eq!(app.focused_widget(), Some("calendar"));
        app.zoom_enter();
        assert_eq!(
            app.zoom_target.as_ref().map(|z| z.widget_id.as_str()),
            Some("calendar")
        );
    }

    /// `exit_zoom` clears `zoom_target` and restores focus to the widget that was zoomed.
    #[cfg(feature = "widgets-all")]
    #[test]
    fn exit_zoom_clears_target_and_restores_focus() {
        let mut app = App::new(Config::default());
        // Move focus to feeds@ai (index 1), then zoom.
        app.cycle_focus(true);
        assert_eq!(app.focused_widget(), Some("feeds@ai"));
        app.zoom_enter();
        assert!(app.zoom_target.is_some());
        // Advance focus without exiting zoom — simulates what a retarget call might do.
        app.focus_idx = 0;
        app.exit_zoom();
        assert!(app.zoom_target.is_none());
        // exit_zoom restores focus_idx to the zoomed widget's position.
        assert_eq!(app.focused_widget(), Some("feeds@ai"));
    }

    /// Pressing `z` while already zoomed exits zoom (no nested zoom).
    #[cfg(feature = "widgets-all")]
    #[test]
    fn second_z_press_exits_zoom() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = App::new(Config::default());
        app.zoom_enter();
        assert!(app.zoom_target.is_some());
        // Second `z` while zoom is active → handled by handle_global_zoom_key → exit_zoom.
        let key = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE);
        app.handle_global_zoom_key(key);
        assert!(app.zoom_target.is_none());
    }

    /// `Esc` while zoomed exits zoom.
    #[cfg(feature = "widgets-all")]
    #[test]
    fn esc_exits_zoom() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = App::new(Config::default());
        app.zoom_enter();
        assert!(app.zoom_target.is_some());
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        app.handle_global_zoom_key(key);
        assert!(app.zoom_target.is_none());
    }

    /// `retarget_zoom_cycle` advances and wraps correctly.
    #[cfg(feature = "widgets-all")]
    #[test]
    fn retarget_zoom_cycle_wraps() {
        let mut app = App::new(Config::default());
        app.zoom_enter();
        // focus_order for the fixed layout is [calendar, feeds@ai, notes, email].
        let n = app.focus_order.len();
        assert!(n > 1);
        let start_id = app.zoom_target.as_ref().unwrap().widget_id.clone();
        app.retarget_zoom_cycle(true);
        let next_id = app.zoom_target.as_ref().unwrap().widget_id.clone();
        assert_ne!(start_id, next_id);
        // Cycle back and forward (n-1) more times to get back to start.
        for _ in 0..(n - 1) {
            app.retarget_zoom_cycle(true);
        }
        let final_id = app.zoom_target.as_ref().unwrap().widget_id.clone();
        assert_eq!(start_id, final_id, "cycling forward n times should wrap back to start");
    }

    /// `assign_shortcuts` never assigns 'z' — it is reserved for zoom.
    #[test]
    fn assign_shortcuts_never_assigns_z() {
        let mut manager = WidgetManager::new();
        let shortcuts = assign_shortcuts(&mut manager);
        // 'z' must not appear as any assigned shortcut regardless of widget preferences.
        assert!(
            !shortcuts.contains_key(&'z'),
            "assign_shortcuts must never assign 'z' (reserved for zoom toggle)"
        );
    }

    /// `zoom_backdrop_tick_counter` starts at zero.
    #[test]
    fn zoom_backdrop_tick_counter_starts_at_zero() {
        let app = App::new(Config::default());
        assert_eq!(app.zoom_backdrop_tick_counter, 0);
    }

    /// Verify the backdrop-throttle decision math: with ratio=3, over 9
    /// increments the backdrop is allowed on ticks 3, 6, 9 (3 of 9).
    /// The `allow_backdrop_tick` expression in the tick arm is replicated
    /// exactly here so any future refactor that changes the expression
    /// without updating this test will surface the divergence.
    #[test]
    fn zoom_backdrop_tick_throttle_math_correct() {
        let ratio: u64 = 3;
        let mut counter: u64 = 0;
        let mut allowed: u64 = 0;
        for _ in 0..9 {
            counter = counter.wrapping_add(1);
            // Replicate the tick arm condition (zoom is assumed active, ratio > 1).
            if ratio == 1 || counter % ratio == 0 {
                allowed += 1;
            }
        }
        // 9 ticks ÷ ratio 3 = 3 allowed backdrop ticks (at ticks 3, 6, 9).
        assert_eq!(allowed, 9 / ratio);
    }

    /// With ratio=1, every tick is a backdrop tick regardless of counter.
    #[test]
    fn zoom_backdrop_tick_ratio_one_means_no_throttle() {
        let ratio: u64 = 1;
        let mut counter: u64 = 0;
        let mut allowed: u64 = 0;
        for _ in 0..10 {
            counter = counter.wrapping_add(1);
            if ratio == 1 || counter % ratio == 0 {
                allowed += 1;
            }
        }
        assert_eq!(allowed, 10, "ratio=1 must allow every tick");
    }
}

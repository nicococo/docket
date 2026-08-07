// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! Email widget — read-only feed of recent messages over IMAP.
//!
//! Closely mirrors the News widget (provider trait, expand/select/open flow,
//! optional LLM summarization, refresh polling). Key differences:
//!   - "Folders" replace News's topic tabs.
//!   - Server-side read state is OR'd with a local "seen via docket" cache
//!     so docket never has to write to the server.
//!   - Bodies come from the provider's body endpoint, with HTML→text fallback.

pub mod html_strip;
pub mod imap;
pub mod provider;
pub mod seen_store;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::text::{pad_or_truncate, truncate, wrap};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Local, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};
use serde::Deserialize;

use crate::cache::ScopedCache;
use crate::llm::{LlmMessage, LlmProvider, LlmRequest, Role};
use crate::theme::{ColorScheme, Theme};
use crate::ui::{apply_title_row, MetadataEmphasis};

use super::{AppContext, EventResult, ViewTier, Widget};

use provider::{EmailMessage, EmailProvider};
use seen_store::SeenStore;

const MAX_SUMMARY_LINES: usize = 5;
const MAX_PER_FOLDER: usize = 100;
/// Implicit first tab in multi-account IMAP mode — merges every configured
/// account, mirroring News's "All" topic tab.
const ALL_ACCOUNTS_TAB: &str = "All";
/// Minimum list-area content width before the list splits into list + read pane.
/// Intentionally very wide: the read pane is only shown when the widget is
/// genuinely large (zoomed pane or a wide dedicated cell). At 175 cols and
/// below the list fills the full width; the read pane only fires at ≥ 176.
/// (Per-widget deviation — see the ViewTier convention-sweep note.)
const READ_PANE_MIN_WIDTH: u16 = 176;

const SUMMARY_SYSTEM_PROMPT: &str = "You are a concise email summarizer. \
Given a sender, subject, and the message body, return a neutral summary in at \
most 4 sentences. Capture the asks, decisions, and dates only. Do not editorialize, \
do not greet, do not use markdown. If the input is too sparse to summarize \
faithfully, respond with the single sentence: \"Insufficient content to summarize.\"";

#[derive(Debug, Clone)]
enum SummaryState {
    Requested,
    Ready(String),
    Failed,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmailConfig {
    /// Only `"imap"` is supported. Anything else renders a placeholder.
    #[serde(default = "default_provider")]
    pub provider: String,

    /// Pull messages received within the last N days.
    #[serde(default = "default_latest_days")]
    pub latest_days: u32,

    #[serde(default = "default_refresh_minutes")]
    pub refresh_minutes: u64,

    /// IMAP folder/mailbox names (`INBOX`, `Sent`, …). Ignored when
    /// `accounts` below is non-empty.
    #[serde(default = "default_folders")]
    pub folders: Vec<String>,

    /// Multiple named IMAP accounts merged into one panel — each becomes
    /// its own tab (plus an implicit "All" tab), analogous to News's topic
    /// tabs. Only meaningful when `provider = "imap"`; empty (the default)
    /// preserves single-account behavior exactly, reading `folders` above
    /// and `credentials/imap.toml`. When non-empty, each entry's
    /// credentials instead live in `credentials/imap_<label>.toml`.
    #[serde(default)]
    pub accounts: Vec<EmailAccountEntry>,

    /// On-demand message summarisation when an LLM provider is configured.
    /// Press `s` on an expanded message.
    #[serde(default)]
    pub summarize_with_llm: bool,

    /// Pre-populates the title's address before the provider's `/me` lookup
    /// resolves. The lookup still runs and overwrites this once it returns.
    #[serde(default)]
    pub account_address: Option<String>,

    #[serde(default)]
    pub colors: ColorScheme,

    /// `Shift+<letter>` focus shortcuts; falls back to the letters in "email".
    #[serde(default)]
    pub shortcuts: Vec<char>,
}

fn default_provider() -> String {
    "imap".into()
}
fn default_latest_days() -> u32 {
    7
}
fn default_refresh_minutes() -> u64 {
    5
}
fn default_folders() -> Vec<String> {
    vec!["INBOX".into()]
}

/// One IMAP account inside a multi-account `[[accounts]]` list.
#[derive(Debug, Clone, Deserialize)]
pub struct EmailAccountEntry {
    /// Tab label, and the suffix of its credentials file
    /// (`credentials/imap_<label>.toml`).
    pub label: String,
    /// Folders to merge for this account. Defaults to `["INBOX"]`, same as
    /// single-account mode.
    #[serde(default = "default_folders")]
    pub folders: Vec<String>,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            latest_days: default_latest_days(),
            refresh_minutes: default_refresh_minutes(),
            folders: default_folders(),
            accounts: Vec::new(),
            summarize_with_llm: false,
            account_address: None,
            colors: ColorScheme::default(),
            shortcuts: Vec::new(),
        }
    }
}

#[derive(Default)]
struct EmailState {
    /// `Arc<EmailMessage>` so the per-render `Vec::clone()` is O(N)
    /// atomic increments instead of O(N) deep EmailMessage copies. A
    /// typical inbox snapshot is 50+ messages × multiple Strings each,
    /// previously cloned wholesale every time the clock-driven 1 Hz
    /// redraw fired.
    messages: Vec<Arc<EmailMessage>>,
    selected: usize,
    scroll: usize,
    expanded: bool,
    /// Index into `folders`. 0 is always the first configured folder.
    active_folder_idx: usize,
    last_error: Option<String>,
    /// Two-tier polling: while `account` is unresolved we retry on a
    /// fast 30 s cadence (capped so a failing profile endpoint
    /// doesn't spin); once the account address lands we fall back
    /// to the configured mail-refresh interval. Both stamp on every
    /// spawn_refresh since `ensure_account` piggybacks on
    /// `fetch_recent`.
    account_poll: crate::polling::PollTracker,
    mail_poll: crate::polling::PollTracker,
    inflight: bool,
    /// Cached account address (e.g. "alice@example.com") for the title row.
    /// Populated lazily from the provider once the first fetch resolves.
    account: Option<String>,
    /// Per-message LLM summarization state, keyed by message id.
    summaries: std::collections::HashMap<String, SummaryState>,
    /// Per-message view preference, keyed by message id. `true` means
    /// "prefer the LLM summary"; missing/`false` means "show the raw
    /// body" (the historical default). Set by `s`: first press flips
    /// to summary (and kicks off the request if needed); subsequent
    /// presses toggle without re-firing the LLM (cached summary is
    /// reused).
    summary_view: std::collections::HashMap<String, bool>,
    /// Last-rendered row layout for the message list: `(msg_idx, row_start, row_end_exclusive)`
    /// in offsets relative to the list_area's top. Populated on every
    /// render so `handle_mouse` can map a click row back to a message
    /// without recomputing wrap heights.
    row_layout: Vec<(usize, u16, u16)>,
    /// Last-rendered list_area Rect — used together with `row_layout` to
    /// translate raw mouse coordinates into a clicked message index.
    last_list_area: Option<Rect>,
    /// True when the last render painted a read pane (list_area.width ≥
    /// READ_PANE_MIN_WIDTH at an Expanded/Full tier). Written by the render
    /// path; read by handle_key to suppress the inline `e`/Enter expand
    /// while the full body is already visible in the read pane.
    read_pane_active: bool,
    /// Armed by `d`; the message pending a delete-to-trash confirmation.
    /// `y` commits ([`EmailWidget::confirm_delete`]), any other key
    /// cancels — see the shared `crate::ui::modal` confirm primitive.
    confirm_delete: Option<Arc<EmailMessage>>,
    /// Display-state dirty bit drained by `take_dirty`. Set true by
    /// every async-task / tick-time mutation site so the main loop's
    /// dirty-flag gate triggers a redraw.
    dirty: bool,
}

const CACHE_KEY_MESSAGES: &str = "messages";

/// Cache key for the resolved account email address. Persisted with a
/// very long TTL since the IMAP username effectively never changes for a
/// configured account.
const CACHE_KEY_ACCOUNT_ADDRESS: &str = "account_address";

/// Cache-key namespace for LLM-generated message summaries. Each summary is
/// keyed by `summary-<sha256(id)>`. Provider IDs are filesystem-safe today
/// but hashing keeps the namespace bounded and future-provider-proof. Email
/// bodies don't change post-delivery so a cached summary is valid until the
/// user explicitly clears the cache.
const SUMMARY_CACHE_PREFIX: &str = "summary-";

fn summary_cache_key(id: &str) -> String {
    crate::cache::short_hash_key(SUMMARY_CACHE_PREFIX, id)
}

pub struct EmailWidget {
    id: String,
    instance: String,
    display_name_cache: String,
    provider: Arc<EmailProviderHandle>,
    state: Arc<Mutex<EmailState>>,
    /// In-memory + on-disk seen-set, shared with the refresh task so it can
    /// react to expand-induced changes without races.
    seen: Arc<Mutex<SeenStore>>,
    folders: Vec<String>,
    /// Non-empty only in multi-account IMAP mode (`[[accounts]]` in
    /// email.toml). When present, this widget ignores `provider`/`folders`
    /// above and fetches from every listed account instead; the tab bar
    /// shows an account per tab (plus "All") rather than folders. See
    /// [`Self::tab_labels`] and [`Self::spawn_refresh_multi_imap`].
    imap_accounts: Vec<ImapAccountHandle>,
    latest_days: u32,
    summarize_with_llm: bool,
    llm: Option<Arc<dyn LlmProvider>>,
    /// "imap" / "none" — drives the bracketed source tag in the title.
    provider_label: String,
    /// True when no real provider was configurable (missing token, missing
    /// client config, unknown name). The widget shows a placeholder instead
    /// of an empty list.
    provider_ready: bool,
    /// Diagnostic surfaced under the placeholder when `provider_ready` is
    /// false. Walk-through tells the user what to run.
    auth_hint: Option<String>,
    app_theme: Arc<Theme>,
    colors_override: ColorScheme,
    theme: Theme,
    shortcut: Option<char>,
    shortcut_prefs: Vec<char>,
    /// Persistent cache of the merged message list across configured folders.
    cache: ScopedCache,
}

/// One connected account in multi-account IMAP mode. Deliberately holds a
/// concrete `imap::ImapProvider` rather than going through
/// `EmailProviderHandle` — multi-account is IMAP-only, so there's no
/// dispatch to do, and `cached_account()` is called directly from `render`.
struct ImapAccountHandle {
    /// Tab label — matches the `[[accounts]]` entry's `label`.
    label: String,
    provider: Arc<imap::ImapProvider>,
    folders: Vec<String>,
}

/// Thin wrapper so the widget can fetch a fresh `cached_account()` snapshot
/// from the provider implementation without having to widen the
/// `EmailProvider` trait.
enum EmailProviderHandle {
    Imap(imap::ImapProvider),
    /// Placeholder used when no provider could be constructed. Holds nothing;
    /// `fetch_recent` returns an empty list so the widget renders a friendly
    /// placeholder instead of crashing.
    Empty,
}

impl EmailProviderHandle {
    fn as_provider(&self) -> Option<&dyn EmailProvider> {
        match self {
            Self::Imap(p) => Some(p),
            Self::Empty => None,
        }
    }

    fn cached_account(&self) -> Option<String> {
        match self {
            Self::Imap(p) => p.cached_account(),
            Self::Empty => None,
        }
    }

    /// Prime the provider's in-memory account cache from a persisted
    /// value (loaded from the on-disk scoped cache or seeded in
    /// email.toml). Skips the next `/me` round-trip so the title row
    /// paints instantly on launch.
    fn seed_account_cache(&self, address: &str) {
        match self {
            Self::Imap(p) => p.seed_account_cache(address),
            Self::Empty => {}
        }
    }
}

impl EmailWidget {
    #[cfg(test)]
    pub fn with_config(config: EmailConfig) -> Self {
        Self::with_config_and_llm(
            "main".to_string(),
            config,
            None,
            Arc::new(Theme::builtin_defaults()),
            ScopedCache::ephemeral(),
        )
    }

    pub fn with_config_and_llm(
        instance: String,
        config: EmailConfig,
        llm: Option<Arc<dyn LlmProvider>>,
        app_theme: Arc<Theme>,
        cache: ScopedCache,
    ) -> Self {
        let folders = if config.folders.is_empty() {
            default_folders()
        } else {
            config.folders.clone()
        };
        let multi_imap = config.provider.eq_ignore_ascii_case("imap") && !config.accounts.is_empty();
        let (provider, provider_label, provider_ready, auth_hint, imap_accounts) = if multi_imap {
            let (handles, ready, hint) = build_imap_accounts(&config.accounts);
            // Multi-account mode never reads `provider` (every code path
            // branches on `imap_accounts` being non-empty first), but the
            // field still needs a value.
            (
                EmailProviderHandle::Empty,
                "imap".to_string(),
                ready,
                hint,
                handles,
            )
        } else {
            let (p, label, ready, hint) = build_provider(&config.provider);
            (p, label, ready, hint, Vec::new())
        };

        let colors_override = config.colors.clone();
        let theme = app_theme.with_overrides(&colors_override);
        let shortcut_prefs = if config.shortcuts.is_empty() {
            vec!['e', 'm', 'a', 'i', 'l']
        } else {
            config.shortcuts.clone()
        };

        let id = if instance == "main" {
            "email".to_string()
        } else {
            format!("email@{instance}")
        };
        let display_name_cache = if instance == "main" {
            "Email".to_string()
        } else {
            format!("Email ({instance})")
        };

        // Seed the seen-store using the provider+account pair. We don't know
        // the account yet (the /me call lands on first refresh), so start
        // with a stable "_unknown_" placeholder file; on the first
        // `update_account_cache` call after a successful fetch we transparently
        // swap to the real per-account file. Worst case: a single session's
        // worth of seen state goes to the placeholder file — a fine trade
        // for keeping the widget responsive on cold start.
        let seen = SeenStore::load(&provider_label, "_unknown_").unwrap_or_else(|err| {
            tracing::warn!(error = %err, "failed to load email seen-store, starting empty");
            // SAFETY: SeenStore::load only fails on disk errors; we fall
            // back to a fresh in-memory-only store by trying again with a
            // tmp tag (which will likely succeed; if not, we accept the
            // panic since we've already logged).
            SeenStore::load(&provider_label, "_unknown_").expect("seen-store fallback failed")
        });

        let poll_interval = Duration::from_secs(config.refresh_minutes.max(1) * 60);
        // Seed messages from cache so the first render shows the prior
        // session's inbox while the refresh runs in the background.
        // The account address has its own long-lived cache entry — the
        // configured IMAP username effectively never changes, so caching
        // it lets the title row paint with the user's email immediately
        // on launch instead of "(loading…)" until the first connection
        // returns. `account_address` in email.toml still wins so users
        // can override the cached value by hand.
        let cached_address = cache
            .load::<String>(CACHE_KEY_ACCOUNT_ADDRESS)
            .map(|e| e.value);
        let initial_account = config.account_address.clone().or(cached_address.clone());
        // Multi-account mode has no single title-row address to resolve —
        // render() reads each account's cached_account() directly instead —
        // so seed a sentinel here purely to keep `is_due()` on the regular
        // mail-refresh cadence instead of the single-account fast retry
        // meant for an unresolved `/me` lookup.
        let multi_imap_account_sentinel = if multi_imap {
            Some("multi-account".to_string())
        } else {
            None
        };
        let mut initial_state = EmailState {
            account: multi_imap_account_sentinel.or(initial_account.clone()),
            // Fast retry while account is being resolved (~30s) plus
            // the regular mail-refresh cadence. Both get stamped on
            // every spawn_refresh; is_due picks which one to consult
            // based on whether `account` has landed yet.
            account_poll: crate::polling::PollTracker::new(Duration::from_secs(30)),
            mail_poll: crate::polling::PollTracker::new(poll_interval),
            ..EmailState::default()
        };
        // Seed the provider's in-memory cache too so the first
        // fetch_recent's `ensure_account` is a no-op and doesn't hit
        // the network just to re-derive what we already know.
        if let Some(addr) = &initial_account {
            provider.seed_account_cache(addr);
        }
        if let Some(entry) = cache.load::<Vec<EmailMessage>>(CACHE_KEY_MESSAGES) {
            initial_state.mail_poll.seed_from_cache_age(entry.age());
            // If we have cached messages, an account-resolution retry
            // shouldn't fire instantly either — pretend account was
            // checked recently so we don't double-hit on launch.
            initial_state.account_poll.seed_from_cache_age(entry.age());
            let mut messages = entry.value;
            for m in &mut messages {
                truncate_body_in_place(&mut m.plain_body, 4096);
            }
            initial_state.messages = messages.into_iter().map(Arc::new).collect();
        }
        // Spread first-fire phases across instances so multiple
        // 60s-cadence widgets don't all hit the network in the same
        // 250ms tick. `account_poll` runs at 30s; jittering both keeps
        // the two-stage startup from synchronising either.
        initial_state
            .mail_poll
            .apply_jitter(&format!("email@{instance}"));
        initial_state
            .account_poll
            .apply_jitter(&format!("email-account@{instance}"));

        Self {
            id,
            instance,
            display_name_cache,
            provider: Arc::new(provider),
            state: Arc::new(Mutex::new(initial_state)),
            seen: Arc::new(Mutex::new(seen)),
            folders,
            imap_accounts,
            latest_days: config.latest_days.max(1),
            summarize_with_llm: config.summarize_with_llm,
            llm,
            provider_label,
            provider_ready,
            auth_hint,
            app_theme,
            colors_override,
            theme,
            shortcut: None,
            shortcut_prefs,
            cache,
        }
    }

    /// Visible tab labels — folders in single-account mode, or `"All"` +
    /// one tab per configured account in multi-account IMAP mode. Every
    /// tab-bar / tab-cycling / tab-filtering call site goes through this
    /// instead of touching `self.folders` directly, so the two modes share
    /// one rendering and input path.
    fn tab_labels(&self) -> Vec<String> {
        if self.imap_accounts.is_empty() {
            self.folders.clone()
        } else {
            let mut tabs = vec![ALL_ACCOUNTS_TAB.to_string()];
            tabs.extend(self.imap_accounts.iter().map(|a| a.label.clone()));
            tabs
        }
    }

    /// Whether `msg` belongs on the given visible tab — folder equality in
    /// single-account mode, account equality (or the "All" tab) in
    /// multi-account mode.
    fn message_matches_tab(&self, msg: &EmailMessage, tab: &str) -> bool {
        if self.imap_accounts.is_empty() {
            msg.folder.eq_ignore_ascii_case(tab)
        } else {
            tab == ALL_ACCOUNTS_TAB || msg.account.eq_ignore_ascii_case(tab)
        }
    }

    fn filtered_messages(&self) -> Vec<Arc<EmailMessage>> {
        let st = self.state.lock().expect("email state poisoned");
        let tabs = self.tab_labels();
        let active = tabs
            .get(st.active_folder_idx.min(tabs.len().saturating_sub(1)))
            .cloned()
            .unwrap_or_default();
        st.messages
            .iter()
            .filter(|m| self.message_matches_tab(m, &active))
            .cloned()
            .collect()
    }

    fn is_due(&self) -> bool {
        let st = self.state.lock().expect("email state poisoned");
        if st.inflight {
            return false;
        }
        // Two-tier policy: while the account address is still being
        // resolved, retry on `account_poll`'s fast 30s cadence so
        // the title row doesn't sit on "(loading…)" for the full
        // mail interval. Once the account lands, switch to the
        // configured mail-refresh interval via `mail_poll`. The
        // tracker's `is_due()` handles the elapsed-check uniformly.
        if st.account.is_none() {
            return st.account_poll.is_due();
        }
        st.mail_poll.is_due()
    }

    fn mark_dirty(&self) {
        let mut st = self.state.lock().expect("email state poisoned");
        // User-triggered refresh: dirty both timers so neither stops
        // the next fetch.
        st.account_poll.mark_dirty();
        st.mail_poll.mark_dirty();
    }

    fn spawn_refresh(&self) {
        if !self.provider_ready {
            return;
        }
        {
            let mut st = self.state.lock().expect("email state poisoned");
            st.inflight = true;
            // ensure_account piggybacks on fetch_recent, so a single
            // refresh advances both timers.
            st.account_poll.mark_attempted();
            st.mail_poll.mark_attempted();
            st.dirty = true;
        }
        if !self.imap_accounts.is_empty() {
            self.spawn_refresh_multi_imap();
            return;
        }
        let provider = self.provider.clone();
        let state = self.state.clone();
        let folders = self.folders.clone();
        let latest_days = self.latest_days;
        let cache = self.cache.clone();
        tokio::spawn(async move {
            let Some(prov) = provider.as_provider() else {
                let mut st = state.lock().expect("email state poisoned");
                st.inflight = false;
                st.dirty = true;
                return;
            };
            let since = Utc::now() - chrono::Duration::days(latest_days as i64);
            let mut messages: Vec<EmailMessage> = Vec::new();
            let mut last_error: Option<String> = None;
            for folder in &folders {
                match prov.fetch_recent(folder, since, MAX_PER_FOLDER).await {
                    Ok(mut chunk) => messages.append(&mut chunk),
                    Err(err) => {
                        tracing::warn!(folder = %folder, error = %err, "email fetch failed");
                        last_error = Some(format!("{folder}: {err}"));
                    }
                }
            }
            // Sort newest-first across all folders.
            messages.sort_by_key(|m| std::cmp::Reverse(m.received));
            // Trim oversized bodies. The expanded view caps at
            // `MAX_SUMMARY_LINES` (5) and full-message read happens via
            // `o` opening the user's mail client, so we never paint
            // more than the first ~400 chars in docket anyway. 4 KB is
            // ample headroom for the visible snippet + LLM summary
            // context, and drops mailing-list bodies that routinely
            // ship 50+ KB of HTML-stripped text per message.
            for m in &mut messages {
                truncate_body_in_place(&mut m.plain_body, 4096);
            }
            // Persist before swapping state so a concurrent reload sees the
            // same payload either way. Errors are warned and ignored.
            if last_error.is_none() {
                if let Err(err) = cache.store(CACHE_KEY_MESSAGES, &messages) {
                    tracing::warn!(error = %err, "email cache store failed");
                }
            }
            // Capture the just-refreshed account address (the providers populate
            // their cache during fetch_recent). Persist it so the next
            // launch paints the title row instantly instead of waiting
            // for `/me` to resolve again.
            let account = provider.cached_account();
            if let Some(addr) = &account {
                if let Err(err) = cache.store(CACHE_KEY_ACCOUNT_ADDRESS, addr) {
                    tracing::warn!(error = %err, "email account-address cache store failed");
                }
            }
            let mut st = state.lock().expect("email state poisoned");
            st.inflight = false;
            st.messages = messages.into_iter().map(Arc::new).collect();
            st.last_error = last_error;
            if account.is_some() {
                st.account = account;
            }
            st.dirty = true;
        });
    }

    /// Multi-account IMAP refresh path — fetches every configured
    /// account's folders in turn (accounts run sequentially; each
    /// account's own folders were already sequential in single-account
    /// mode, so this keeps the same "no concurrent IMAP sessions to worry
    /// about" property) and merges the results, tagging each message with
    /// its account label. Mirrors [`Self::spawn_refresh`]'s body but
    /// without the single-account title-row address bookkeeping — the
    /// title row reads each account's `cached_account()` directly in
    /// `render` instead.
    fn spawn_refresh_multi_imap(&self) {
        let accounts: Vec<(String, Arc<imap::ImapProvider>, Vec<String>)> = self
            .imap_accounts
            .iter()
            .map(|a| (a.label.clone(), a.provider.clone(), a.folders.clone()))
            .collect();
        let state = self.state.clone();
        let latest_days = self.latest_days;
        let cache = self.cache.clone();
        tokio::spawn(async move {
            let since = Utc::now() - chrono::Duration::days(latest_days as i64);
            let mut messages: Vec<EmailMessage> = Vec::new();
            let mut last_error: Option<String> = None;
            for (label, prov, folders) in &accounts {
                for folder in folders {
                    match prov.fetch_recent(folder, since, MAX_PER_FOLDER).await {
                        Ok(mut chunk) => {
                            // Tag with the account label (used by tab
                            // filtering) and disambiguate the id — IMAP UIDs
                            // are only unique per-account, so two accounts'
                            // "INBOX" could otherwise collide in the
                            // seen-store and the message-list keying.
                            for m in &mut chunk {
                                m.account = label.clone();
                                m.id = format!("{label}-{}", m.id);
                            }
                            messages.append(&mut chunk);
                        }
                        Err(err) => {
                            tracing::warn!(account = %label, folder = %folder, error = %err, "email fetch failed");
                            last_error = Some(format!("{label}/{folder}: {err}"));
                        }
                    }
                }
            }
            messages.sort_by_key(|m| std::cmp::Reverse(m.received));
            for m in &mut messages {
                truncate_body_in_place(&mut m.plain_body, 4096);
            }
            if last_error.is_none() {
                if let Err(err) = cache.store(CACHE_KEY_MESSAGES, &messages) {
                    tracing::warn!(error = %err, "email cache store failed");
                }
            }
            let mut st = state.lock().expect("email state poisoned");
            st.inflight = false;
            st.messages = messages.into_iter().map(Arc::new).collect();
            st.last_error = last_error;
            st.dirty = true;
        });
    }

    fn move_selection(&mut self, delta: isize) {
        let filtered = self.filtered_messages();
        if filtered.is_empty() {
            return;
        }
        let mut st = self.state.lock().expect("email state poisoned");
        let new_idx = (st.selected as isize + delta).clamp(0, filtered.len() as isize - 1) as usize;
        st.selected = new_idx;
        // Scrolling/selecting must never mark a message read — only the
        // explicit `u` keybinding (toggle_read_state) changes read state.
    }

    fn jump_to(&mut self, idx: usize) {
        let filtered = self.filtered_messages();
        if filtered.is_empty() {
            return;
        }
        let mut st = self.state.lock().expect("email state poisoned");
        st.selected = idx.min(filtered.len() - 1);
    }

    /// Press-`u` entry point: flip the selected message between read and
    /// unread. Updates the local seen-store immediately (so the UI reacts
    /// with no network latency), then fires an async IMAP `STORE` to push
    /// the same state to the server — see [`Self::spawn_set_seen`].
    fn toggle_read_state(&mut self) {
        let filtered = self.filtered_messages();
        let selected: Option<Arc<EmailMessage>> = {
            let st = self.state.lock().expect("email state poisoned");
            filtered.get(st.selected).cloned()
        };
        let Some(msg) = selected else {
            return;
        };
        let currently_unread = self.is_unread(&msg);
        let mut seen = self.seen.lock().expect("seen-store poisoned");
        let result = if currently_unread {
            seen.mark_seen(&msg.id)
        } else {
            seen.mark_unread(&msg.id)
        };
        if let Err(err) = result {
            tracing::warn!(error = %err, id = %msg.id, "failed to persist read-state toggle");
        }
        drop(seen);
        // New server state is the opposite of "currently unread".
        self.spawn_set_seen(&msg, /* seen */ currently_unread);
        let mut st = self.state.lock().expect("email state poisoned");
        st.dirty = true;
    }

    /// Fire-and-forget: push `seen` to the mail server for `msg` over
    /// IMAP. No-op (with a warning) for messages without a known
    /// `imap_uid` — messages from before this field existed in a stale
    /// cache. Best-effort: on
    /// failure the local seen-store override already applied by the
    /// caller stands, so the toggle still "works" from the user's
    /// perspective, just without server-side reflection until the next
    /// successful toggle or manual retry.
    fn spawn_set_seen(&self, msg: &Arc<EmailMessage>, seen: bool) {
        let Some(uid) = msg.imap_uid else {
            tracing::warn!(id = %msg.id, "no imap_uid on message; can't write read-state to server");
            return;
        };
        // Multi-account IMAP mode holds concrete `Arc<imap::ImapProvider>`
        // handles (see `ImapAccountHandle`) rather than going through
        // `EmailProviderHandle`, so the two modes need separate lookups.
        if !self.imap_accounts.is_empty() {
            let Some(account) = self.imap_accounts.iter().find(|a| a.label == msg.account) else {
                tracing::warn!(id = %msg.id, account = %msg.account, "no matching imap account to write read-state to server");
                return;
            };
            let provider = account.provider.clone();
            let folder = msg.folder.clone();
            let id = msg.id.clone();
            tokio::spawn(async move {
                if let Err(err) = provider.set_seen(&folder, uid, seen).await {
                    tracing::warn!(error = %err, id = %id, seen, "failed to write read-state to server");
                }
            });
            return;
        }
        let provider = self.provider.clone();
        let folder = msg.folder.clone();
        let id = msg.id.clone();
        tokio::spawn(async move {
            let Some(prov) = provider.as_provider() else {
                return;
            };
            if let Err(err) = prov.set_seen(&folder, uid, seen).await {
                tracing::warn!(error = %err, id = %id, seen, "failed to write read-state to server");
            }
        });
    }

    /// Press-`d` entry point: arm the delete-to-trash confirmation modal
    /// for the selected message. No-op if nothing is selected.
    fn arm_delete_confirm(&mut self) {
        let filtered = self.filtered_messages();
        let mut st = self.state.lock().expect("email state poisoned");
        if let Some(msg) = filtered.get(st.selected) {
            st.confirm_delete = Some(msg.clone());
        }
    }

    /// User answered `y` on the delete-to-trash modal: optimistically
    /// remove the message from the visible list (so the UI reacts with
    /// no network latency), then fire the actual IMAP move-to-Trash in
    /// the background — see [`Self::spawn_move_to_trash`].
    fn confirm_delete(&mut self) {
        let msg = {
            let mut st = self.state.lock().expect("email state poisoned");
            st.confirm_delete.take()
        };
        let Some(msg) = msg else { return };
        {
            let mut st = self.state.lock().expect("email state poisoned");
            st.messages.retain(|m| m.id != msg.id);
            st.selected = st.selected.min(st.messages.len().saturating_sub(1));
            st.dirty = true;
        }
        self.spawn_move_to_trash(&msg);
    }

    /// Fire-and-forget: move `msg` to the server's Trash. Mirrors
    /// [`Self::spawn_set_seen`]'s provider dispatch (multi-account IMAP
    /// holds concrete `Arc<imap::ImapProvider>` handles; single-account
    /// mode goes through `EmailProviderHandle`).
    ///
    /// On failure, re-inserts `msg` into the visible list — the
    /// optimistic removal in `confirm_delete` was premature — and
    /// surfaces the error via the existing `last_error` banner. Silently
    /// swallowing a failed delete would leave the user believing a
    /// message is safely in Trash when it's actually still sitting
    /// untouched on the server.
    fn spawn_move_to_trash(&self, msg: &Arc<EmailMessage>) {
        let Some(uid) = msg.imap_uid else {
            tracing::warn!(id = %msg.id, "no imap_uid on message; can't delete on server");
            let mut st = self.state.lock().expect("email state poisoned");
            st.messages.push(msg.clone());
            st.messages.sort_by_key(|m| std::cmp::Reverse(m.received));
            st.last_error = Some(
                "Delete failed: message has no server id yet — try again after the next refresh"
                    .into(),
            );
            st.dirty = true;
            return;
        };
        let state = self.state.clone();
        let msg = msg.clone();
        if !self.imap_accounts.is_empty() {
            let Some(account) = self.imap_accounts.iter().find(|a| a.label == msg.account) else {
                tracing::warn!(id = %msg.id, account = %msg.account, "no matching imap account to delete on server");
                return;
            };
            let provider = account.provider.clone();
            let folder = msg.folder.clone();
            tokio::spawn(async move {
                if let Err(err) = provider.move_to_trash(&folder, uid).await {
                    tracing::warn!(error = %err, id = %msg.id, "failed to delete message on server");
                    let mut st = state.lock().expect("email state poisoned");
                    st.messages.push(msg);
                    st.messages.sort_by_key(|m| std::cmp::Reverse(m.received));
                    st.last_error = Some(format!("Delete failed: {err}"));
                    st.dirty = true;
                }
            });
            return;
        }
        let provider = self.provider.clone();
        let folder = msg.folder.clone();
        tokio::spawn(async move {
            let Some(prov) = provider.as_provider() else {
                return;
            };
            if let Err(err) = prov.move_to_trash(&folder, uid).await {
                tracing::warn!(error = %err, id = %msg.id, "failed to delete message on server");
                let mut st = state.lock().expect("email state poisoned");
                st.messages.push(msg);
                st.messages.sort_by_key(|m| std::cmp::Reverse(m.received));
                st.last_error = Some(format!("Delete failed: {err}"));
                st.dirty = true;
            }
        });
    }

    fn cycle_folder(&mut self, forward: bool) {
        let n = self.tab_labels().len();
        if n <= 1 {
            return;
        }
        let mut st = self.state.lock().expect("email state poisoned");
        st.active_folder_idx = if forward {
            (st.active_folder_idx + 1) % n
        } else {
            (st.active_folder_idx + n - 1) % n
        };
        st.selected = 0;
        st.scroll = 0;
        st.expanded = false;
    }

    fn open_selected(&self) {
        let filtered = self.filtered_messages();
        let url = {
            let st = self.state.lock().expect("email state poisoned");
            filtered.get(st.selected).and_then(|m| m.web_url.clone())
        };
        if let Some(url) = url {
            if let Err(err) = open::that(&url) {
                tracing::warn!(error = %err, url = %url, "failed to open email URL");
            }
        }
    }

    /// Toggle expanded state on the selected message. Does *not* change
    /// read state — only the explicit `u` keybinding
    /// ([`Self::toggle_read_state`]) does that.
    fn toggle_expand(&mut self) {
        let mut st = self.state.lock().expect("email state poisoned");
        if st.messages.is_empty() {
            return;
        }
        st.expanded = !st.expanded;
    }

    /// Press-`s` entry point. Drives the per-message Body ⇄ Summary
    /// toggle with a side-effect of expanding when the user hits it from
    /// collapsed mode. Never changes read state — only `u`
    /// ([`Self::toggle_read_state`]) does that.
    ///
    /// - **Collapsed**: expand, switch to Summary view, fire the LLM
    ///   (cache-hit returns instantly).
    /// - **Expanded + currently Body**: switch to Summary; if not yet
    ///   requested, fire the LLM (cache-hit returns instantly).
    /// - **Expanded + currently Summary**: switch back to Body — no
    ///   LLM call, no state mutation beyond the view-pref flip.
    fn toggle_summary_view(&mut self) {
        if !self.summarize_with_llm || self.llm.is_none() {
            return;
        }
        let filtered = self.filtered_messages();
        let selected: Option<Arc<EmailMessage>> = {
            let st = self.state.lock().expect("email state poisoned");
            filtered.get(st.selected).cloned()
        };
        let Some(msg) = selected else {
            return;
        };

        let will_show_summary = {
            let mut st = self.state.lock().expect("email state poisoned");
            let was_collapsed = !st.expanded;
            if was_collapsed {
                st.expanded = true;
                st.summary_view.insert(msg.id.clone(), true);
                true
            } else {
                let cur = *st.summary_view.get(&msg.id).unwrap_or(&false);
                let new = !cur;
                st.summary_view.insert(msg.id.clone(), new);
                new
            }
        };

        if will_show_summary {
            // request_summary is idempotent — cache-hits jump straight
            // to Ready without an LLM call. Calling unconditionally
            // here is safe + cheap.
            self.request_summary();
        }
    }

    fn request_summary(&self) {
        if !self.summarize_with_llm || self.llm.is_none() {
            return;
        }
        let filtered = self.filtered_messages();
        let selected: Option<Arc<EmailMessage>> = {
            let st = self.state.lock().expect("email state poisoned");
            filtered.get(st.selected).cloned()
        };
        let Some(msg) = selected else {
            return;
        };
        {
            let st = self.state.lock().expect("email state poisoned");
            if st.summaries.contains_key(&msg.id) {
                return;
            }
        }
        let cache_key = summary_cache_key(&msg.id);
        if let Some(entry) = self.cache.load::<String>(&cache_key) {
            let mut st = self.state.lock().expect("email state poisoned");
            st.summaries
                .insert(msg.id.clone(), SummaryState::Ready(entry.value));
            st.dirty = true;
            return;
        }
        let Some(llm) = self.llm.clone() else {
            return;
        };
        let state = self.state.clone();
        let cache = self.cache.clone();
        {
            let mut st = self.state.lock().expect("email state poisoned");
            st.summaries.insert(msg.id.clone(), SummaryState::Requested);
            st.dirty = true;
        }
        let id = msg.id.clone();
        let body = msg.plain_body.clone();
        let subject = msg.subject.clone();
        let from = format_sender(&msg.from_name, &msg.from_address);
        tokio::spawn(async move {
            let request = LlmRequest {
                model: None,
                system: Some(SUMMARY_SYSTEM_PROMPT.into()),
                messages: vec![LlmMessage {
                    role: Role::User,
                    content: format!(
                        "From: {from}\nSubject: {subject}\n\n{}",
                        if body.is_empty() {
                            "(empty body)"
                        } else {
                            body.as_str()
                        }
                    ),
                }],
                max_tokens: 300,
                cache_system: true,
            };
            let outcome = match llm.complete(request).await {
                Ok(resp) => {
                    let text = resp.text.trim();
                    if text
                        .to_ascii_lowercase()
                        .starts_with("insufficient content to summarize")
                    {
                        SummaryState::Failed
                    } else {
                        SummaryState::Ready(text.to_string())
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, id = %id, "LLM email summary failed");
                    SummaryState::Failed
                }
            };
            if let SummaryState::Ready(text) = &outcome {
                if let Err(err) = cache.store(&cache_key, text) {
                    tracing::warn!(error = %err, id = %id, "email summary cache store failed");
                }
            }
            let mut st = state.lock().expect("email state poisoned");
            st.summaries.insert(id, outcome);
            st.dirty = true;
        });
    }

    /// True if the message should display the unread `●` indicator.
    /// Priority: an explicit `u`-forced-unread override always wins; next a
    /// "seen via docket" mark (auto-set on expand, or via `u`) always reads
    /// as read; otherwise falls back to the server's own unread state.
    fn is_unread(&self, msg: &EmailMessage) -> bool {
        let seen = self.seen.lock().expect("seen-store poisoned");
        if seen.is_forced_unread(&msg.id) {
            return true;
        }
        if seen.contains(&msg.id) {
            return false;
        }
        msg.server_unread
    }

    /// Mirrors the inner-area split used by `render`.
    fn split_inner(&self, inner: Rect) -> (Rect, Rect, Rect) {
        let has_tabs = self.tab_labels().len() > 1;
        let tab_height: u16 = if has_tabs { 2 } else { 1 };
        let footer_height = 1u16;
        let list_height = inner.height.saturating_sub(footer_height + tab_height);
        let tab_area = Rect::new(inner.x, inner.y, inner.width, tab_height);
        let list_area = Rect::new(inner.x, inner.y + tab_height, inner.width, list_height);
        let footer_area = Rect::new(
            inner.x,
            inner.y + inner.height.saturating_sub(footer_height),
            inner.width,
            footer_height,
        );
        (tab_area, list_area, footer_area)
    }

    fn tab_index_at(&self, click_col: u16, tab_area: Rect) -> Option<usize> {
        let mut x: u16 = tab_area.x + 1;
        for (i, label) in self.tab_labels().iter().enumerate() {
            let w = label.chars().count() as u16 + 2;
            if click_col >= x && click_col < x + w {
                return Some(i);
            }
            x += w + 1;
            if x >= tab_area.x + tab_area.width {
                break;
            }
        }
        None
    }
}

/// Build an `EmailProviderHandle` from the configured provider name. Returns
/// `(handle, label, ready, hint)` where `ready=false` means the widget should
/// render the placeholder; `hint` is the actionable next step shown to the user.
fn build_provider(name: &str) -> (EmailProviderHandle, String, bool, Option<String>) {
    match name.to_ascii_lowercase().as_str() {
        "imap" => match build_imap("imap.toml") {
            Ok(p) => (EmailProviderHandle::Imap(p), "imap".into(), true, None),
            Err(hint) => (EmailProviderHandle::Empty, "imap".into(), false, Some(hint)),
        },
        other => (
            EmailProviderHandle::Empty,
            other.to_string(),
            false,
            Some(format!("unknown provider {other:?} (expected imap)")),
        ),
    }
}

fn build_imap(filename: &str) -> Result<imap::ImapProvider, String> {
    let dir = crate::credentials::dir()
        .map_err(|err| format!("IMAP credentials dir unavailable: {err}"))?;
    let path = dir.join(filename);
    if !path.exists() {
        return Err(format!(
            "IMAP credentials missing at {} — fill it in by hand (see README)",
            path.display()
        ));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("read {} failed: {err}", path.display()))?;
    let creds: imap::ImapCredentials =
        toml::from_str(&text).map_err(|err| format!("parse {} failed: {err}", path.display()))?;
    if creds.username.trim().is_empty() || creds.app_password.trim().is_empty() {
        return Err(format!(
            "{} has empty username or app_password — edit and retry",
            path.display()
        ));
    }
    Ok(imap::ImapProvider::new(creds))
}

/// Build one [`ImapAccountHandle`] per `[[accounts]]` entry, each loading
/// its own `credentials/imap_<label>.toml`. Partial failure is tolerated —
/// accounts that fail to load are dropped with a hint appended to the
/// returned diagnostic string; the widget still renders whichever accounts
/// did connect. Returns `ready = true` as soon as at least one succeeds.
fn build_imap_accounts(entries: &[EmailAccountEntry]) -> (Vec<ImapAccountHandle>, bool, Option<String>) {
    let mut handles = Vec::new();
    let mut hints = Vec::new();
    for entry in entries {
        let filename = format!("imap_{}.toml", entry.label);
        match build_imap(&filename) {
            Ok(p) => handles.push(ImapAccountHandle {
                label: entry.label.clone(),
                provider: Arc::new(p),
                folders: if entry.folders.is_empty() {
                    default_folders()
                } else {
                    entry.folders.clone()
                },
            }),
            Err(hint) => hints.push(format!("{}: {hint}", entry.label)),
        }
    }
    let ready = !handles.is_empty();
    let hint = if hints.is_empty() {
        None
    } else {
        Some(hints.join(" · "))
    };
    (handles, ready, hint)
}

// ── Widget trait impl ───────────────────────────────────────────────────────

#[async_trait]
impl Widget for EmailWidget {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &str {
        "email"
    }

    fn instance(&self) -> &str {
        &self.instance
    }

    fn display_name(&self) -> &str {
        &self.display_name_cache
    }

    async fn update(&mut self, _ctx: &AppContext) -> Result<()> {
        if self.is_due() {
            self.spawn_refresh();
        }
        Ok(())
    }

    fn take_dirty(&mut self) -> bool {
        let mut st = self.state.lock().expect("email state poisoned");
        std::mem::replace(&mut st.dirty, false)
    }

    fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let (messages, selected, mut scroll, expanded, active_idx, inflight, last_error, account) = {
            let st = self.state.lock().expect("email state poisoned");
            (
                st.messages.clone(),
                st.selected,
                st.scroll,
                st.expanded,
                st.active_folder_idx,
                st.inflight,
                st.last_error.clone(),
                st.account.clone(),
            )
        };

        // Apply the active tab filter (folder in single-account mode,
        // account in multi-account IMAP mode — see `tab_labels`).
        let tabs = self.tab_labels();
        let active_tab = tabs
            .get(active_idx.min(tabs.len().saturating_sub(1)))
            .cloned()
            .unwrap_or_default();
        let filtered: Vec<Arc<EmailMessage>> = messages
            .into_iter()
            .filter(|m| self.message_matches_tab(m, &active_tab))
            .collect();

        // Base title is just "Email" / "Email (instance)" — the
        // provider + account address are metadata, rendered via the
        // shared title-with-metadata helper for consistency with
        // other widgets.
        let base = if self.instance == "main" {
            "Email".to_string()
        } else {
            format!("Email ({})", self.instance)
        };
        let metadata = if self.imap_accounts.is_empty() {
            let account_label = account
                .as_deref()
                .map(String::from)
                .unwrap_or_else(|| "(loading…)".into());
            format!("[{}] {}", self.provider_label, account_label)
        } else {
            // No single resolved address to show — list every account's
            // username instead (`cached_account()` is just the configured
            // IMAP username, available immediately with no round-trip).
            let names: Vec<String> = self
                .imap_accounts
                .iter()
                .map(|a| a.provider.cached_account().unwrap_or_else(|| a.label.clone()))
                .collect();
            format!("[imap] {}", names.join(", "))
        };

        let block = apply_title_row(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(self.theme.border_style(focused)),
            focused,
            &base,
            Some(metadata.as_str()),
            MetadataEmphasis::Default,
            self.shortcut,
            &self.theme,
            area.width,
        );
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let tier = ViewTier::from_rect(area);

        let (tab_area, list_area, footer_area) = self.split_inner(inner);

        // At Expanded/Full tiers with enough width, split the list area into:
        //   left (50%) | 3-col gutter | right (remaining)
        // The gutter renders a centered `│` with one blank column on each side.
        // Below READ_PANE_MIN_WIDTH the list column is too narrow, so
        // Compact/Standard (and cramped Expanded) leave the read pane off (None).
        let (list_area, gutter_area, read_area) = if tier >= ViewTier::Expanded
            && list_area.width >= READ_PANE_MIN_WIDTH
        {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(50),
                    Constraint::Length(3),
                    Constraint::Min(0),
                ])
                .split(list_area);
            (chunks[0], Some(chunks[1]), Some(chunks[2]))
        } else {
            (list_area, None, None)
        };

        // Placeholder when no provider is configured (no token, etc.).
        if !self.provider_ready {
            let mut lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Email provider not connected.",
                    self.theme.text_brilliant,
                )),
            ];
            if let Some(hint) = &self.auth_hint {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(hint.clone(), self.theme.text_dim)));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Run `docket --setup` to configure email.",
                self.theme.text_dim,
            )));
            let body = Paragraph::new(lines).alignment(Alignment::Center);
            frame.render_widget(body, inner);
            return;
        }

        // Tab bar — folders in single-account mode, accounts in
        // multi-account IMAP mode.
        let has_tabs = tabs.len() > 1;
        if has_tabs {
            let mut spans: Vec<Span<'_>> = Vec::with_capacity(tabs.len() * 2);
            spans.push(Span::raw(" "));
            for (i, label) in tabs.iter().enumerate() {
                let is_active = i == active_idx;
                let style = if is_active {
                    self.theme.text_selected
                } else {
                    self.theme.text_dim
                };
                spans.push(Span::styled(format!("[{label}]"), style));
                if i + 1 < tabs.len() {
                    spans.push(Span::raw(" "));
                }
            }
            frame.render_widget(Paragraph::new(Line::from(spans)), tab_area);
        }

        if filtered.is_empty() {
            let msg = if inflight {
                "Loading messages…".to_string()
            } else if let Some(err) = last_error.as_ref() {
                format!("Last fetch failed: {err}")
            } else {
                "No recent messages.".to_string()
            };
            let body =
                Paragraph::new(vec![Line::from(""), Line::from(msg)]).alignment(Alignment::Center);
            frame.render_widget(body, inner);
            return;
        }

        // Layout each message row:
        //   ●  Alice Smith            Re: Project update                                12:43
        // When expanded: subject + body/summary lines underneath. The
        // expansion height is variable (depends on whether the user is
        // viewing the raw body or the LLM summary, and on the wrapped
        // line count), so we measure it explicitly below.
        const ROWS_PER_ITEM: usize = 1;
        let list_height = list_area.height;
        let items_visible = (list_height as usize / ROWS_PER_ITEM).max(1);
        // Baseline: keep the selected message in view.
        if selected < scroll {
            scroll = selected;
        }
        if selected >= scroll + items_visible {
            scroll = selected + 1 - items_visible;
        }
        // Extra: if expanded, scroll up far enough that the full
        // expansion (subject + body/summary) fits below the selected
        // row when possible. Clamps so the selected row never scrolls
        // off the top — for emails whose expansion exceeds the pane
        // height, the selected row pins to the top and the bottom of
        // the expansion clips (the standard "long content" failure
        // mode; the user can collapse or use the LLM summary to
        // shorten).
        if expanded && read_area.is_none() {
            if let Some(msg) = filtered.get(selected) {
                let body_max_width = (list_area.width as usize).saturating_sub(3);
                let subject_lines = wrap_text(&msg.subject, body_max_width, 2).len();
                let body_lines = expanded_body_lines(
                    msg,
                    &self.state,
                    body_max_width,
                    self.summarize_with_llm && self.llm.is_some(),
                    MAX_SUMMARY_LINES,
                )
                .len();
                let expansion_height = subject_lines + body_lines;
                let want = (selected + 1 + expansion_height).saturating_sub(items_visible);
                scroll = scroll.max(want).min(selected);
            }
        }

        let now_local = Local::now();
        // Reserve a 1-cell right buffer so the timestamp column
        // doesn't run flush against the widget's right border.
        // All column-width math below derives from `inner_width`,
        // so shrinking it here automatically gives the row its
        // trailing gutter without touching the per-row span list.
        let inner_width = (list_area.width as usize).saturating_sub(1);
        // Column-width policy:
        //   * Date is fixed at 8 chars (matches the formats produced by
        //     `format_received`: "Fri 14:25", "Yesterday", "Mar 03", …).
        //   * Sender label is 20 chars by default, growing up to 25 when
        //     a wide pane leaves surplus space — long names like
        //     "alex.thompson@example.com" become legible on a roomy
        //     display without crowding subjects on a narrow one.
        //   * Subject text is capped at 95 visible chars (anything past
        //     that scans worse than it reads). Surplus pane width past
        //     that cap first feeds sender, then becomes trailing padding
        //     between subject and date — which keeps the date right-
        //     aligned no matter how wide the pane gets.
        //   * Indicator (●/○) + space prefix = 2 chars, and there are
        //     two single-space inter-column gaps → 4 chars of fixed
        //     chrome on every row.
        const SENDER_LABEL_MIN: usize = 20;
        const SENDER_LABEL_MAX: usize = 25;
        const SUBJECT_TEXT_MAX: usize = 95;
        const DATE_COL_W: usize = 8;
        const INDICATOR_PREFIX_W: usize = 2;
        const COL_GAPS_W: usize = 2;

        let mut sender_label_w = SENDER_LABEL_MIN;
        let mut sender_col_w = sender_label_w + INDICATOR_PREFIX_W;
        let mut subject_col_w = inner_width.saturating_sub(sender_col_w + DATE_COL_W + COL_GAPS_W);
        // When subject would overflow the 95-char cap, donate the excess
        // to sender first (up to SENDER_LABEL_MAX). Any remaining surplus
        // stays in the subject column as trailing padding so the date
        // column hugs the right edge.
        if subject_col_w > SUBJECT_TEXT_MAX {
            let excess = subject_col_w - SUBJECT_TEXT_MAX;
            let donate = excess.min(SENDER_LABEL_MAX - SENDER_LABEL_MIN);
            sender_label_w += donate;
            sender_col_w = sender_label_w + INDICATOR_PREFIX_W;
            subject_col_w = inner_width.saturating_sub(sender_col_w + DATE_COL_W + COL_GAPS_W);
        }
        let date_col_w = DATE_COL_W;
        let subject_text_w = subject_col_w.min(SUBJECT_TEXT_MAX);

        let mut lines: Vec<Line<'_>> = Vec::with_capacity(items_visible);
        let mut rows_emitted: u16 = 0;
        let mut row_layout: Vec<(usize, u16, u16)> = Vec::new();
        for (i, msg) in filtered.iter().enumerate().skip(scroll) {
            let row_start = rows_emitted;
            let is_selected = i == selected;
            let expand_this = is_selected && expanded && read_area.is_none();

            let unread = self.is_unread(msg);
            // Read messages fall back to the same dim/non-bold style as
            // the date column — only unread messages get the brilliant/
            // focused treatment. Selection highlight always wins so the
            // cursor stays visible regardless of read state.
            let row_style = if is_selected {
                self.theme.text_selected
            } else if !unread {
                self.theme.text_dim
            } else if focused {
                self.theme.text_focused
            } else {
                self.theme.text_brilliant
            };

            let indicator = if unread { "●" } else { "○" };
            let sender = normalize_sender(&msg.from_name, &msg.from_address, sender_label_w);
            let date = format_received(now_local, msg.received);
            let subject = if msg.subject.is_empty() {
                "(no subject)".to_string()
            } else {
                msg.subject.clone()
            };
            // Truncate the subject text at the cap, then pad the column
            // out to its full width so the date column stays pinned to
            // the right edge regardless of how much surplus space the
            // pane has past 95 chars of subject.
            let subject_truncated = truncate(&subject, subject_text_w);

            let sender_padded = pad_or_truncate(&sender, sender_label_w);
            let subject_padded = pad_or_truncate(&subject_truncated, subject_col_w);
            let date_padded = format!("{date:>w$}", w = date_col_w);

            let row = Line::from(vec![
                Span::styled(
                    format!("{indicator} "),
                    if unread {
                        self.theme.text_focused
                    } else {
                        self.theme.text_dim
                    },
                ),
                Span::styled(sender_padded, row_style),
                Span::raw(" "),
                Span::styled(subject_padded, row_style),
                Span::raw(" "),
                Span::styled(date_padded, self.theme.text_dim),
            ]);
            lines.push(row);
            rows_emitted += 1;

            if expand_this {
                let body_lines = expanded_body_lines(
                    msg,
                    &self.state,
                    inner_width.saturating_sub(3),
                    self.summarize_with_llm && self.llm.is_some(),
                    MAX_SUMMARY_LINES,
                );
                // First the full subject on its own row(s) (up to 2).
                for sline in wrap_text(&msg.subject, inner_width.saturating_sub(3), 2) {
                    if rows_emitted >= list_height {
                        break;
                    }
                    lines.push(Line::from(Span::styled(
                        format!("   {sline}"),
                        self.theme.text_brilliant,
                    )));
                    rows_emitted += 1;
                }
                for bline in &body_lines {
                    if rows_emitted >= list_height {
                        break;
                    }
                    lines.push(Line::from(Span::styled(
                        format!("   {bline}"),
                        Style::default(),
                    )));
                    rows_emitted += 1;
                }
            }

            row_layout.push((i, row_start, rows_emitted));
            if rows_emitted >= list_height {
                break;
            }
        }
        frame.render_widget(Paragraph::new(lines), list_area);

        // Expanded/Full tier: render the selected message's full body in the
        // right-hand read pane. `read_area` is None at Compact/Standard tier
        // so this block is a no-op there.
        if let Some(rp) = read_area {
            let mut rp_lines: Vec<Line<'_>> = Vec::new();
            match filtered.get(selected) {
                None => {
                    // Empty selection (e.g. folder just switched, selected
                    // index not yet clamped). Show a dim placeholder.
                    rp_lines.push(Line::from(""));
                    rp_lines.push(Line::from(Span::styled(
                        "Select a message",
                        self.theme.text_dim,
                    )));
                }
                Some(msg) => {
                    // Header row 1: sender (left-aligned) + date (right-aligned).
                    let date_str = format_received(now_local, msg.received);
                    let sender_budget =
                        (rp.width as usize).saturating_sub(date_str.len() + 2).max(1);
                    let sender_display =
                        normalize_sender(&msg.from_name, &msg.from_address, sender_budget);
                    let sender_padded = pad_or_truncate(&sender_display, sender_budget);
                    rp_lines.push(Line::from(vec![
                        Span::styled(sender_padded, self.theme.text_focused),
                        Span::raw("  "),
                        Span::styled(date_str, self.theme.text_dim),
                    ]));
                    // Header row 2: full subject, truncated to pane width.
                    let subject_display = if msg.subject.is_empty() {
                        "(no subject)".to_string()
                    } else {
                        msg.subject.clone()
                    };
                    rp_lines.push(Line::from(Span::styled(
                        truncate(&subject_display, rp.width as usize),
                        self.theme.text_brilliant,
                    )));
                    // Blank separator between header and body.
                    rp_lines.push(Line::from(""));
                    // Body: full message with no MAX_SUMMARY_LINES cap,
                    // wrapped to pane width. Honors the per-message summary
                    // toggle (`s`) — when the user has switched into summary
                    // view this renders the already-generated summary (reused
                    // from the in-memory/disk cache, no fresh LLM call)
                    // instead of the raw body. Clip to available rows with a
                    // trailing dim "…" when the content overflows the pane.
                    const HEADER_ROWS: usize = 3; // sender, subject, blank
                    let body_rows_avail = (rp.height as usize).saturating_sub(HEADER_ROWS);
                    if body_rows_avail > 0 {
                        let body_w = (rp.width as usize).saturating_sub(1).max(1);
                        let all_body = expanded_body_lines(
                            msg,
                            &self.state,
                            body_w,
                            self.summarize_with_llm && self.llm.is_some(),
                            usize::MAX,
                        );
                        let clipped = all_body.len() > body_rows_avail;
                        let take = if clipped {
                            body_rows_avail.saturating_sub(1)
                        } else {
                            body_rows_avail
                        };
                        for bline in all_body.iter().take(take) {
                            rp_lines.push(Line::from(Span::raw(bline.clone())));
                        }
                        if clipped {
                            rp_lines.push(Line::from(Span::styled("…", self.theme.text_dim)));
                        }
                    }
                }
            }
            frame.render_widget(Paragraph::new(rp_lines), rp);
        }

        // Vertical divider between the list pane and the read pane. The gutter
        // is 3 cols wide (from the Layout above); `│` sits in the center column
        // with one blank on each side. Styled dim to match other widget borders.
        if let Some(gutter) = gutter_area {
            let divider_lines: Vec<Line<'_>> = (0..gutter.height)
                .map(|_| {
                    Line::from(vec![
                        Span::raw(" "),
                        Span::styled("│", self.theme.text_dim),
                        Span::raw(" "),
                    ])
                })
                .collect();
            frame.render_widget(Paragraph::new(divider_lines), gutter);
        }

        // Hide the `s summarize` hint when summarisation isn't usable —
        // either the user disabled it in email.toml or there's no LLM
        // key configured. Surfacing an unbindable key in the footer is
        // confusing ("I pressed s and nothing happened…").
        let summarize_usable = self.summarize_with_llm && self.llm.is_some();
        // When the read pane is active the `e`/Enter key is a no-op, so drop
        // that hint to avoid implying a binding that does nothing in this mode.
        let footer_text = if read_area.is_some() {
            if summarize_usable {
                "↑/↓ select · ←/→ folder · o open · s summarize · u read/unread · d delete · r refresh"
            } else {
                "↑/↓ select · ←/→ folder · o open · u read/unread · d delete · r refresh"
            }
        } else if summarize_usable {
            "↑/↓ select · ←/→ folder · e/⏎/click expand · o open · s summarize · u read/unread · d delete · r refresh"
        } else {
            "↑/↓ select · ←/→ folder · e/⏎/click expand · o open · u read/unread · d delete · r refresh"
        };
        let footer = Paragraph::new(Line::from(Span::styled(footer_text, self.theme.text_dim)))
            .alignment(Alignment::Right);
        frame.render_widget(footer, footer_area);

        // Persist scroll + the row layout so click handling can map
        // mouse coordinates back to a message index.
        let confirm_target = {
            let mut st = self.state.lock().expect("email state poisoned");
            st.scroll = scroll;
            st.row_layout = row_layout;
            st.last_list_area = Some(list_area);
            st.read_pane_active = read_area.is_some();
            st.confirm_delete.clone()
        };
        if let Some(msg) = confirm_target {
            let subject = if msg.subject.trim().is_empty() {
                "(no subject)".to_string()
            } else {
                msg.subject.clone()
            };
            crate::ui::modal::render(
                frame,
                area,
                &self.theme,
                crate::ui::modal::ConfirmModal {
                    title: " Move to Trash? ",
                    target: &subject,
                    hint: Some("  [y] delete (recoverable ~30d)  ·  any other key cancels"),
                    max_width: 54,
                },
            );
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> EventResult {
        if key.modifiers != KeyModifiers::NONE && key.modifiers != KeyModifiers::SHIFT {
            return EventResult::Ignored;
        }
        // Uppercase ASCII letters are reserved for the app-wide
        // `Shift+<letter>` focus-jump dispatcher — never consume them here.
        // This is why jump-to-bottom is `End`, not the vim-style `G`.
        if let KeyCode::Char(c) = key.code {
            if c.is_ascii_uppercase() {
                return EventResult::Ignored;
            }
        }
        // Delete-to-trash confirm modal: y commits, any other key
        // cancels. Handled before the normal dispatch so the user can't
        // accidentally move selection / open a message while the
        // prompt is up.
        if self
            .state
            .lock()
            .expect("email state poisoned")
            .confirm_delete
            .is_some()
        {
            match crate::ui::modal::dispatch_key(key) {
                crate::ui::modal::ConfirmChoice::Confirm => self.confirm_delete(),
                crate::ui::modal::ConfirmChoice::Cancel => {
                    self.state
                        .lock()
                        .expect("email state poisoned")
                        .confirm_delete = None;
                }
            }
            return EventResult::Handled;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                EventResult::Handled
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                EventResult::Handled
            }
            KeyCode::PageUp => {
                self.move_selection(-10);
                EventResult::Handled
            }
            KeyCode::PageDown => {
                self.move_selection(10);
                EventResult::Handled
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.jump_to(0);
                EventResult::Handled
            }
            KeyCode::End => {
                self.jump_to(usize::MAX);
                EventResult::Handled
            }
            KeyCode::Char('e') | KeyCode::Enter => {
                let read_pane_active = {
                    let st = self.state.lock().expect("email state poisoned");
                    st.read_pane_active
                };
                if !read_pane_active {
                    self.toggle_expand();
                }
                EventResult::Handled
            }
            KeyCode::Char('o') => {
                self.open_selected();
                EventResult::Handled
            }
            KeyCode::Char('s') => {
                self.toggle_summary_view();
                EventResult::Handled
            }
            KeyCode::Char('r') => {
                self.mark_dirty();
                EventResult::Handled
            }
            KeyCode::Char('u') => {
                self.toggle_read_state();
                EventResult::Handled
            }
            KeyCode::Char('d') => {
                self.arm_delete_confirm();
                EventResult::Handled
            }
            KeyCode::Char('[') | KeyCode::Left | KeyCode::Char('h') => {
                self.cycle_folder(false);
                EventResult::Handled
            }
            KeyCode::Char(']') | KeyCode::Right | KeyCode::Char('l') => {
                self.cycle_folder(true);
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) -> EventResult {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.move_selection(-1);
                return EventResult::Handled;
            }
            MouseEventKind::ScrollDown => {
                self.move_selection(1);
                return EventResult::Handled;
            }
            MouseEventKind::Down(MouseButton::Left) => {}
            _ => return EventResult::Ignored,
        }
        if area.width < 2 || area.height < 2 {
            return EventResult::Ignored;
        }
        let inner = Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2);
        let (tab_area, _list_area, _footer_area) = self.split_inner(inner);
        if tab_area.height > 0
            && mouse.row == tab_area.y
            && mouse.column >= tab_area.x
            && mouse.column < tab_area.x + tab_area.width
        {
            if let Some(idx) = self.tab_index_at(mouse.column, tab_area) {
                let mut st = self.state.lock().expect("email state poisoned");
                if st.active_folder_idx != idx {
                    st.active_folder_idx = idx;
                    st.selected = 0;
                    st.scroll = 0;
                    st.expanded = false;
                }
                return EventResult::Handled;
            }
        }
        // Click inside the message list — find the row that owns this
        // mouse position and toggle expand on that message (selecting it
        // first if it wasn't already the active row). Hit-test against the
        // last-rendered list area, which is the narrowed left column when a
        // read pane is present, so a click landing in the read pane is a
        // no-op rather than jumping to whatever row sits at that vertical
        // offset.
        let list_area = {
            let st = self.state.lock().expect("email state poisoned");
            st.last_list_area
        };
        let in_list = list_area.is_some_and(|la| {
            la.height > 0
                && mouse.column >= la.x
                && mouse.column < la.x + la.width
                && mouse.row >= la.y
                && mouse.row < la.y + la.height
        });
        if let (true, Some(list_area)) = (in_list, list_area) {
            let click_offset = mouse.row - list_area.y;
            let hit_and_state = {
                let st = self.state.lock().expect("email state poisoned");
                st.row_layout
                    .iter()
                    .find(|(_, start, end)| click_offset >= *start && click_offset < *end)
                    .map(|(idx, _, _)| (*idx, st.selected))
            };
            if let Some((idx, selected_before)) = hit_and_state {
                if idx != selected_before {
                    // Switch selection first, then force-expand via toggle.
                    // Setting expanded=false beforehand makes toggle_expand
                    // flip to true and run the mark-as-seen side effect.
                    let mut st = self.state.lock().expect("email state poisoned");
                    st.selected = idx;
                    st.expanded = false;
                    drop(st);
                    self.toggle_expand();
                } else {
                    self.toggle_expand();
                }
                return EventResult::Handled;
            }
        }
        EventResult::Ignored
    }

    fn handle_command(&mut self, cmd: &str, _args: &[&str]) -> Result<bool> {
        match cmd {
            "email" => Ok(true),
            "refresh" => {
                self.mark_dirty();
                Ok(false) // let the global :refresh dispatch continue
            }
            _ => Ok(false),
        }
    }

    fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("↑ / ↓ / j / k", "select message"),
            (
                "← / → / [ / ] / h / l",
                "cycle folder (or account, in multi-account IMAP mode)",
            ),
            ("PgUp / PgDn", "±10 messages"),
            ("g / Home", "jump to top"),
            ("End", "jump to bottom"),
            ("e / Enter / click", "expand selected"),
            ("o", "open message in browser"),
            ("s", "request LLM summary (when enabled)"),
            ("u", "toggle read/unread (IMAP: syncs to server)"),
            ("d", "delete to Trash — recoverable ~30d (IMAP only, y to confirm)"),
            ("r", "force refresh"),
        ]
    }

    fn config(&self) -> serde_json::Value {
        let mail_secs = self
            .state
            .lock()
            .expect("email state poisoned")
            .mail_poll
            .interval()
            .as_secs();
        serde_json::json!({
            "provider": self.provider_label,
            "latest_days": self.latest_days,
            "refresh_minutes": mail_secs / 60,
            "folders": self.folders,
            "summarize_with_llm": self.summarize_with_llm,
        })
    }

    fn apply_config(&mut self, config: serde_json::Value) -> Result<()> {
        let new_config: EmailConfig =
            serde_json::from_value(config).context("invalid email config payload")?;
        let llm = self.llm.clone();
        let app_theme = self.app_theme.clone();
        let cache = self.cache.clone();
        let instance = self.instance.clone();
        *self = Self::with_config_and_llm(instance, new_config, llm, app_theme, cache);
        Ok(())
    }

    fn set_app_theme(&mut self, theme: Arc<Theme>) {
        self.theme = theme.with_overrides(&self.colors_override);
        self.app_theme = theme;
    }

    /// Return whichever tracker is currently in effect — account
    /// resolution while we're still waiting for the address,
    /// otherwise the configured mail-refresh cadence — so the
    /// platform sees the cadence actually driving us right now.
    fn poll_snapshot(&self) -> Option<crate::polling::PollSnapshot> {
        let st = self.state.lock().expect("email state poisoned");
        let snap = if st.account.is_none() {
            st.account_poll.snapshot()
        } else {
            st.mail_poll.snapshot()
        };
        Some(snap)
    }

    fn shortcut_preferences(&self) -> &[char] {
        &self.shortcut_prefs
    }

    fn set_shortcut(&mut self, shortcut: Option<char>) {
        self.shortcut = shortcut;
    }

    fn shortcut(&self) -> Option<char> {
        self.shortcut
    }

    fn title_metadata(&self) -> Option<String> {
        // Match the standalone email title's suffix: `[imap]
        // alice@example.com` when the account has resolved; just
        // `[imap]` until then.
        let label = self.provider_label.as_str();
        if label.is_empty() {
            return None;
        }
        let account = self.state.lock().ok().and_then(|st| st.account.clone());
        match account {
            Some(addr) => Some(format!("[{label}] {addr}")),
            None => Some(format!("[{label}]")),
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Normalize a "Name <addr>" pair into a clean display name capped at
/// `max_len` chars. Falls back to the username portion of the address when
/// no display name is present.
pub(crate) fn normalize_sender(name: &Option<String>, address: &str, max_len: usize) -> String {
    let display = match name {
        Some(n) if !n.trim().is_empty() => n.trim().trim_matches('"').to_string(),
        _ => address.split('@').next().unwrap_or(address).to_string(),
    };
    truncate(&display, max_len)
}

fn format_sender(name: &Option<String>, address: &str) -> String {
    match name {
        Some(n) if !n.trim().is_empty() => format!("{n} <{address}>"),
        _ => address.to_string(),
    }
}

fn format_received(now: DateTime<Local>, received: DateTime<Local>) -> String {
    if now.date_naive() == received.date_naive() {
        received.format("%H:%M").to_string()
    } else {
        received.format("%m/%d").to_string()
    }
}

/// Truncate `s` so it occupies at most `max` *terminal cells* (not code
/// points). Wide glyphs (CJK, most emoji) report a width of 2 via
/// Cap `body`'s in-memory length at `max_chars`, appending a brief "…"
/// marker so a future reader notices the truncation. Operates in place
/// to avoid a clone on the common no-op path. Char-boundary safe.
fn truncate_body_in_place(body: &mut String, max_chars: usize) {
    if body.chars().count() <= max_chars {
        return;
    }
    let cutoff = body
        .char_indices()
        .nth(max_chars.saturating_sub(2))
        .map(|(i, _)| i)
        .unwrap_or(body.len());
    body.truncate(cutoff);
    body.push_str("…");
}

/// Body/summary lines for a message, honoring the per-message summary
/// preference (`summary_view`) and any already-generated summary
/// (`summaries`, populated from the in-memory map or the disk cache —
/// never a fresh LLM call). `body_max_lines` caps the *raw body* view:
/// the compact list/expanded panes pass `MAX_SUMMARY_LINES` to keep
/// long emails from crowding the list, while the wide read pane passes
/// `usize::MAX` and does its own row-clipping. LLM summaries are always
/// uncapped (already bounded by the system prompt).
fn expanded_body_lines(
    msg: &EmailMessage,
    state: &Arc<Mutex<EmailState>>,
    max_width: usize,
    llm_enabled: bool,
    body_max_lines: usize,
) -> Vec<String> {
    let (summary_state, prefer_summary) = {
        let st = state.lock().expect("email state poisoned");
        (
            st.summaries.get(&msg.id).cloned(),
            *st.summary_view.get(&msg.id).unwrap_or(&false),
        )
    };
    // Show the summary only when the user has explicitly toggled into
    // summary view for this message (via `s`). The historical default
    // — "always prefer summary if cached" — caused a `s` press to
    // appear to do nothing because the view was already on the
    // cached summary. With the per-message preference, the user gets
    // a predictable Body ⇄ Summary toggle and never loses the
    // original body view to a stale summary.
    if llm_enabled && prefer_summary {
        if let Some(s) = summary_state {
            match s {
                // Ready summaries render in full — the system prompt
                // caps the LLM output at ~4 sentences, so the line
                // count is naturally bounded.
                SummaryState::Ready(text) => {
                    return wrap_text(&text, max_width, usize::MAX);
                }
                SummaryState::Requested => {
                    let mut out = vec!["Summarizing…".to_string()];
                    if !msg.plain_body.is_empty() {
                        out.extend(wrap_text(
                            &msg.plain_body,
                            max_width,
                            body_max_lines.saturating_sub(1),
                        ));
                    }
                    return out;
                }
                // Failed → fall through to body so the user always
                // sees something readable even when the LLM bailed.
                SummaryState::Failed => {}
            }
        }
    }
    // Body view: cap at `body_max_lines`. In the list/expanded panes
    // this is MAX_SUMMARY_LINES (5) — long raw emails would otherwise
    // push every other message off-screen and require multi-pane
    // scrolling. The read pane passes usize::MAX and clips to its own
    // available rows. The LLM summary above stays uncapped (already
    // bounded by the system prompt's ~4 sentences); users who want the
    // full body open the message in their mail client via `o` instead.
    wrap_text(&msg.plain_body, max_width, body_max_lines)
}

/// Thin wrapper preserving the call sites' `wrap_text` name. The
/// canonical implementation lives in [`crate::text::wrap`]; email
/// always wants paragraph-preservation since `\n` in `msg.plain_body`
/// separates real paragraphs.
fn wrap_text(text: &str, max_width: usize, max_lines: usize) -> Vec<String> {
    wrap(text, max_width, max_lines, true)
}

pub const KIND: &str = "email";

pub fn build(ctx: &super::WidgetCtx) -> Box<dyn super::Widget> {
    let cfg: EmailConfig =
        crate::config::load_widget_toml_for_instance(KIND, &ctx.instance).unwrap_or_default();
    Box::new(EmailWidget::with_config_and_llm(
        ctx.instance.clone(),
        cfg,
        ctx.llm.clone(),
        ctx.theme.clone(),
        ctx.cache.clone(),
    ))
}

#[cfg(test)]
mod tests;

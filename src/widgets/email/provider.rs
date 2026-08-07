// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! Common types shared by both Email providers (Gmail + Outlook). The widget
//! talks to providers exclusively through this trait so adding a third
//! provider (IMAP, JMAP, …) later is a strictly additive change.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};

/// A single normalized email message. Provider-specific bodies and headers
/// are reduced to plain text before reaching the widget; everything renderable
/// is on this struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailMessage {
    /// Provider-specific id. Used as the key in the local seen-store and as
    /// the trailing segment of `web_url` (for Gmail).
    pub id: String,
    /// Which folder this message was fetched from. The widget uses this to
    /// group messages under the active folder tab.
    pub folder: String,
    pub from_name: Option<String>,
    pub from_address: String,
    pub subject: String,
    /// Receive time in the user's local zone — providers return UTC; we
    /// normalize at the boundary so the render path can do `%H:%M` /
    /// `%m/%d` formatting without doing TZ math.
    pub received: DateTime<Local>,
    /// Server-side unread state. The widget OR's this with the local
    /// seen-store to decide which messages still warrant the `●` indicator.
    pub server_unread: bool,
    /// Plain-text body. When the source was HTML-only, this is the output of
    /// `html_strip::html_to_text`.
    pub plain_body: String,
    /// Direct URL into the provider's web UI for this message, if available.
    /// Gmail: built from the id. Outlook: comes from Graph's `webLink`. IMAP
    /// (future) will be `None` — there's no canonical web URL for raw IMAP.
    pub web_url: Option<String>,
    /// Which configured account this came from — only meaningful in
    /// multi-account IMAP mode (`[[accounts]]` in email.toml), where it
    /// becomes the tab-filter key alongside `folder`. Providers themselves
    /// leave this empty; the widget stamps it in after `fetch_recent`
    /// returns, since only the widget knows which account label a given
    /// fetch belongs to.
    #[serde(default)]
    pub account: String,
    /// The IMAP UID for this message, when it came from the IMAP
    /// provider — needed to write the `\Seen` flag back to the server
    /// via [`EmailProvider::set_seen`]. `None` for Gmail/Outlook OAuth
    /// messages, which don't support server-side writes in docket yet.
    #[serde(default)]
    pub imap_uid: Option<u32>,
}

/// One folder / label in the user's mailbox. `id` is what the provider
/// expects on its API; `label` is what we show in the tab bar.
#[derive(Debug, Clone)]
#[allow(dead_code)] // surfaced when folder picker UI lands.
pub struct EmailFolder {
    pub label: String,
    pub id: String,
}

/// Read-only email source. v1 has two implementations: Gmail and Outlook.
#[async_trait]
pub trait EmailProvider: Send + Sync {
    /// List the folders/labels available on the account. Used by `--setup`
    /// and future UI to help the user pick which folders to follow.
    #[allow(dead_code)] // surfaced by the wizard / folder picker later.
    async fn list_folders(&self) -> Result<Vec<EmailFolder>>;

    /// Fetch recent messages from a single folder. `since` is a hard
    /// lower bound on `receivedDateTime`; `max` is the hard upper bound
    /// on returned count (caller passes 100).
    async fn fetch_recent(
        &self,
        folder: &str,
        since: DateTime<Utc>,
        max: usize,
    ) -> Result<Vec<EmailMessage>>;

    /// Static label used as the bracketed source tag in the widget title
    /// (e.g. "gmail", "outlook"). The widget builds its own label from the
    /// configured provider name; this method is for diagnostics / future
    /// auto-detection use cases.
    #[allow(dead_code)]
    fn provider_label(&self) -> &str;

    /// Account address (user's primary email) — fetched lazily on the first
    /// successful refresh and cached. Returns `None` before that round-trip
    /// has resolved, in which case the widget shows "(loading…)".
    /// Callers use the concrete `cached_account()` method on each provider
    /// implementation instead, because returning `&str` from behind a Mutex
    /// isn't safely expressible here.
    #[allow(dead_code)]
    fn account_address(&self) -> Option<&str>;

    /// Write the message's read/unread state back to the server, so
    /// pressing `u` in docket is reflected in the real mailbox (and other
    /// clients) rather than being a purely local overlay. `uid` is
    /// [`EmailMessage::imap_uid`]; `seen` is the desired new state.
    ///
    /// Default: unsupported. Only the IMAP provider currently overrides
    /// this — Gmail/Outlook OAuth would need their own API calls
    /// (`users.messages.modify` / Graph's `PATCH .../messages/{id}`)
    /// which aren't wired up.
    async fn set_seen(&self, folder: &str, uid: u32, seen: bool) -> Result<()> {
        let _ = (folder, uid, seen);
        anyhow::bail!("server-side read/unread not supported by this provider")
    }

    /// Move a message to the account's Trash — a *recoverable* delete
    /// (Gmail and most providers auto-purge Trash ~30 days later, and
    /// the message can be manually restored from there any time before
    /// that). `uid` is [`EmailMessage::imap_uid`].
    ///
    /// Default: unsupported, same rationale as [`Self::set_seen`].
    async fn move_to_trash(&self, folder: &str, uid: u32) -> Result<()> {
        let _ = (folder, uid);
        anyhow::bail!("delete-to-trash not supported by this provider")
    }
}

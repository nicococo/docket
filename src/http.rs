// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! Shared HTTP client. Building a [`reqwest::Client`] allocates a fresh
//! TLS session pool — separate clients can't reuse keepalive sockets or
//! cached TLS sessions across widgets. A single process-wide client folds
//! those costs into one pool and lets connection reuse work end-to-end.
//!
//! ## When to share, when to bespoke
//!
//! Use [`shared`] for plain JSON over HTTPS with a docket user-agent —
//! news, weather, calendar (Google + Outlook), email (Gmail + Outlook),
//! LLM providers, geolocation, OAuth flows.
//!
//! Keep a bespoke client for callers needing client-scoped state the
//! shared instance can't carry:
//! - **Cookie store** (`cookie_store(true)`): Yahoo's stocks / forex
//!   endpoints set CSRF cookies on the chart API that the next request
//!   must echo back. A shared cookie store would bleed those cookies
//!   into unrelated widgets.
//! - **Default headers**: CalDAV uses HTTP Basic on every request via
//!   `default_headers(Authorization: …)` — that header would leak into
//!   every other caller if applied on the shared client.

use std::sync::OnceLock;
use std::time::Duration;

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Process-wide reqwest client. Lazily constructed on first call; later
/// calls return cheap clones (Client is internally `Arc`).
///
/// Carries a 30-second timeout — generous enough for slow LLM
/// completions (Anthropic + OpenAI both can take >20s under load), tight
/// enough that a wedged TCP connection won't hang a widget refresh
/// forever. Callers needing a shorter bound (geolocation, weather) apply
/// it per-request via `RequestBuilder::timeout`.
pub fn shared() -> reqwest::Client {
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent(concat!("docket-tui/", env!("CARGO_PKG_VERSION")))
                .timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client should build with default features")
        })
        .clone()
}

/// Browser-shaped User-Agent for per-request overrides on RSS / article-page
/// fetches that need to pass lighter bot-detection gates (Cloudflare,
/// Reddit, …). Deliberately **not** a `(compatible; docket-tui/…; +url)`
/// self-declaring crawler string — several sites (Reddit chief among them)
/// specifically penalize that classic RFC-crawler pattern harder than an
/// anonymous browser UA, even though both eventually hit the same per-IP
/// rate limits. Paired with the Accept/Sec-Fetch-* headers a real browser
/// sends on a top-level navigation; see callers in `widgets::news::provider`,
/// `widgets::feeds::provider`, and `widgets::news::fetch_and_extract_body`.
pub const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:120.0) Gecko/20100101 Firefox/120.0";

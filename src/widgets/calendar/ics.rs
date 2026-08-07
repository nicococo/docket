// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! Plain ICS/webcal feed provider — a single HTTP GET against a static
//! `.ics` URL, no CalDAV discovery, no OAuth. Built for Google Calendar's
//! "Secret address in iCal format" (Settings → a calendar under "Settings
//! for my calendars" → "Integrate calendar"), but works for any calendar
//! that publishes a public or secret iCalendar export.
//!
//! Trade-off vs. the OAuth `google` provider: this is read-only and Google
//! only regenerates the feed every few hours server-side, so it's not
//! real-time. In exchange, there's no Google Cloud project, no OAuth
//! consent screen, and no per-app registration — just a URL.
//!
//! VEVENT parsing is shared with the CalDAV provider (`super::caldav`) —
//! a `.ics` HTTP response and a CalDAV `calendar-data` payload are both
//! plain iCalendar text.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Local};
use serde::Deserialize;

use super::caldav::parse_ics_events;
use super::provider::{CalendarProvider, Event};

/// One or more named feeds, loaded from `credentials/ics.toml`. Each
/// `label` matches a `[[providers]]` block in calendar.toml via that
/// block's `account` field (omitted ⇒ `"default"`, same convention as
/// every other provider kind).
#[derive(Debug, Clone, Deserialize)]
pub struct IcsCredentials {
    #[serde(default)]
    pub feeds: Vec<IcsFeed>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IcsFeed {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub url: String,
}

impl IcsCredentials {
    /// Load `credentials/ics.toml`, dropping any feed that's still the
    /// placeholder template (empty or `REPLACE_WITH_`-prefixed URL) so an
    /// unedited seed file reads as "nothing configured" rather than a
    /// feed with a broken URL.
    pub fn load() -> Result<Option<Self>> {
        let Some(mut creds): Option<Self> = crate::credentials::load("ics.toml")? else {
            return Ok(None);
        };
        drop_placeholder_feeds(&mut creds.feeds);
        if creds.feeds.is_empty() {
            return Ok(None);
        }
        Ok(Some(creds))
    }
}

fn drop_placeholder_feeds(feeds: &mut Vec<IcsFeed>) {
    feeds.retain(|f| !f.url.is_empty() && !f.url.starts_with("REPLACE_WITH_"));
}

pub struct IcsProvider {
    http: reqwest::Client,
    url: String,
    /// Display label for this feed — becomes the `calendar` tag on every
    /// event it produces (drives per-calendar color assignment).
    label: String,
}

impl IcsProvider {
    pub fn new(url: String, label: String) -> Self {
        Self {
            http: crate::http::shared(),
            url,
            label,
        }
    }
}

#[async_trait]
impl CalendarProvider for IcsProvider {
    async fn fetch_range(
        &self,
        start: DateTime<Local>,
        end: DateTime<Local>,
    ) -> Result<Vec<Event>> {
        let resp = self
            .http
            .get(&self.url)
            .send()
            .await
            .with_context(|| format!("ICS feed fetch failed: {}", self.label))?;
        if !resp.status().is_success() {
            let status = resp.status();
            anyhow::bail!("ICS feed {} returned {status}", self.label);
        }
        let body = resp
            .text()
            .await
            .with_context(|| format!("reading ICS feed body: {}", self.label))?;
        // Unlike CalDAV's time-range REPORT filter, a static .ics export
        // has no server-side range query — filter to the requested window
        // ourselves after parsing.
        let events = parse_ics_events(&body, &self.label)
            .into_iter()
            .filter(|e| e.overlaps(start, end))
            .collect();
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(label: &str, url: &str) -> IcsFeed {
        IcsFeed {
            label: label.into(),
            url: url.into(),
        }
    }

    #[test]
    fn drop_placeholder_feeds_removes_unedited_template_entries() {
        let mut feeds = vec![
            feed("default", "REPLACE_WITH_YOUR_SECRET_ICS_URL"),
            feed("empty", ""),
            feed("work", "https://calendar.google.com/calendar/ical/x/basic.ics"),
        ];
        drop_placeholder_feeds(&mut feeds);
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].label, "work");
    }

    #[test]
    fn drop_placeholder_feeds_keeps_all_configured_entries() {
        let mut feeds = vec![
            feed("work", "https://example.com/work.ics"),
            feed("personal", "https://example.com/personal.ics"),
        ];
        drop_placeholder_feeds(&mut feeds);
        assert_eq!(feeds.len(), 2);
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! Local calendar provider — events read from a real `.ics` file
//! (default `~/.config/docket/calendar.ics`, overridable via
//! `[calendar] local_ics_path` in `config.toml`) rather than
//! docket-proprietary TOML. Standard iCalendar means the same file
//! can be imported into (or subscribed from, if synced somewhere
//! reachable) Google Calendar or any other app — the whole point of
//! this format choice over the TOML `[[calendar.events]]` this
//! replaced.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Local};
use std::path::{Path, PathBuf};

use super::caldav::parse_ics_events;
use super::provider::{CalendarProvider, Event};

/// Expand a leading `~/` (or bare `~`) against `$HOME`. Anything else is
/// returned unchanged — `~user/...` is intentionally not supported.
fn expand_tilde(raw: &str) -> PathBuf {
    if raw == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

/// Resolve the local `.ics` path: the configured override (tilde-
/// expanded) if non-empty, else `<config_dir>/calendar.ics`.
pub fn resolve_ics_path(configured: Option<&str>) -> Result<PathBuf> {
    let configured = configured.map(str::trim).filter(|s| !s.is_empty());
    if let Some(p) = configured {
        return Ok(expand_tilde(p));
    }
    Ok(crate::config::config_dir()?.join("calendar.ics"))
}

pub struct LocalCalendarProvider {
    events: Vec<Event>,
}

impl LocalCalendarProvider {
    /// Parse events out of an on-disk `.ics` file. A missing file is
    /// not an error — first run, or nothing's been added yet — it
    /// just yields an empty provider.
    pub fn from_ics_file(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::empty());
            }
            Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
        };
        let mut events = parse_ics_events(&text, "local");
        for e in &mut events {
            // `parse_ics_events` hardcodes "caldav" as the source
            // (shared with the CalDAV/webcal providers) — override it
            // here so color-assignment and the title-row label read
            // "local", matching every other provider's own identity.
            e.source = "local".into();
        }
        Ok(Self { events })
    }

    pub fn empty() -> Self {
        Self { events: Vec::new() }
    }
}

#[async_trait]
impl CalendarProvider for LocalCalendarProvider {
    async fn fetch_range(
        &self,
        start: DateTime<Local>,
        end: DateTime<Local>,
    ) -> Result<Vec<Event>> {
        let mut filtered: Vec<Event> = self
            .events
            .iter()
            .filter(|e| e.overlaps(start, end))
            .cloned()
            .collect();
        filtered.sort_by_key(|e| e.start);
        Ok(filtered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_ics_path_defaults_under_config_dir_when_unset() {
        let _cfg = crate::widgets::test_support::IsolatedConfigHome::new();
        let path = resolve_ics_path(None).unwrap();
        assert!(path.ends_with("calendar.ics"));
        assert_eq!(path.parent(), crate::config::config_dir().ok().as_deref());
    }

    #[test]
    fn resolve_ics_path_expands_tilde_when_set() {
        let path = resolve_ics_path(Some("~/my-calendar.ics")).unwrap();
        assert!(!path.to_string_lossy().contains('~'));
        assert!(path.ends_with("my-calendar.ics"));
    }

    #[tokio::test]
    async fn fetch_range_filters_and_sorts() {
        let ics = "BEGIN:VCALENDAR\nVERSION:2.0\n\
                   BEGIN:VEVENT\nUID:a@docket\nDTSTART:20260520T150000Z\nDTEND:20260520T160000Z\nSUMMARY:Afternoon\nEND:VEVENT\n\
                   BEGIN:VEVENT\nUID:b@docket\nDTSTART:20260520T090000Z\nDTEND:20260520T100000Z\nSUMMARY:Morning\nEND:VEVENT\n\
                   BEGIN:VEVENT\nUID:c@docket\nDTSTART:20260601T090000Z\nDTEND:20260601T100000Z\nSUMMARY:Next month\nEND:VEVENT\n\
                   END:VCALENDAR\n";
        let dir = std::env::temp_dir().join(format!(
            "docket-local-ics-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("calendar.ics");
        std::fs::write(&path, ics).unwrap();

        let p = LocalCalendarProvider::from_ics_file(&path).unwrap();
        let start = chrono::TimeZone::with_ymd_and_hms(&Local, 2026, 5, 20, 0, 0, 0).unwrap();
        let end = chrono::TimeZone::with_ymd_and_hms(&Local, 2026, 5, 21, 0, 0, 0).unwrap();
        let got = p.fetch_range(start, end).await.unwrap();
        assert_eq!(got.len(), 2);
        assert!(got[0].start < got[1].start);
        assert!(got.iter().all(|e| e.source == "local"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_ics_file_yields_empty_provider_not_an_error() {
        let path = std::env::temp_dir().join(format!(
            "docket-local-ics-missing-{}-{:?}.ics",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        let p = LocalCalendarProvider::from_ics_file(&path).unwrap();
        assert!(p.events.is_empty());
    }
}

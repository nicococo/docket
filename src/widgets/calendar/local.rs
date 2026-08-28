// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use serde::Deserialize;

use super::provider::{CalendarProvider, Event};

/// Schema for `~/.config/docket/calendar.toml`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct LocalCalendarFile {
    #[serde(default)]
    pub events: Vec<RawEvent>,
}

/// One row in `[[events]]`. Either timestamps must be RFC3339 (e.g.
/// `2026-05-20T09:30:00-07:00`) for timed events, or plain `YYYY-MM-DD` dates
/// for all-day events.
#[derive(Debug, Clone, Deserialize)]
pub struct RawEvent {
    pub title: String,
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default = "default_calendar")]
    pub calendar: String,
    #[serde(default)]
    pub location: Option<String>,
}

fn default_calendar() -> String {
    "default".into()
}

impl RawEvent {
    fn parse(self) -> Result<Event> {
        let (start, end, all_day) = if self.all_day || is_bare_date(&self.start) {
            let s = parse_local_date(&self.start)
                .with_context(|| format!("invalid start date {:?}", self.start))?;
            let e = parse_local_date(&self.end)
                .with_context(|| format!("invalid end date {:?}", self.end))?;
            // For an all-day event ending on date D, treat the end as the
            // beginning of D+1 so single-day events still have non-zero length.
            let e_exclusive = e
                .checked_add_signed(chrono::Duration::days(1))
                .context("date overflow extending all-day end")?;
            (s, e_exclusive, true)
        } else {
            let s = DateTime::parse_from_rfc3339(&self.start)
                .with_context(|| format!("invalid RFC3339 start {:?}", self.start))?
                .with_timezone(&Local);
            let e = DateTime::parse_from_rfc3339(&self.end)
                .with_context(|| format!("invalid RFC3339 end {:?}", self.end))?
                .with_timezone(&Local);
            (s, e, false)
        };
        Ok(Event {
            title: self.title,
            start,
            end,
            all_day,
            source: "local".into(),
            calendar: self.calendar,
            location: self.location,
        })
    }
}

fn is_bare_date(s: &str) -> bool {
    s.len() == 10 && NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

fn parse_local_date(s: &str) -> Result<DateTime<Local>> {
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d")?;
    let midnight = date
        .and_hms_opt(0, 0, 0)
        .context("date had no midnight (clock change?)")?;
    Local
        .from_local_datetime(&midnight)
        .single()
        .context("ambiguous local time at midnight")
}

pub struct LocalCalendarProvider {
    events: Vec<Event>,
}

impl LocalCalendarProvider {
    pub fn from_file(file: LocalCalendarFile) -> Result<Self> {
        let mut events = Vec::with_capacity(file.events.len());
        for raw in file.events {
            events.push(raw.parse()?);
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

// ── Email-extract add/remove integration ───────────────────────────
//
// Lets the Email widget's "extract dates" AI popup action add/remove
// a local all-day event (see `email::extract_actions`), without a
// general cross-widget dependency — this file only needs to know
// calendar.toml's path and the `[[events]]` shape above. Every write
// is plain text, not a TOML parse/edit: `add_event` appends a new
// `[[events]]` block (always valid regardless of what else is in the
// file, no parser needed), and `remove_event` only ever deletes a
// block this same code wrote, bounded by its own marker comment and
// the next blank line — it never touches anything else in the file.
// Docket's config file-watcher (`config::watcher`) picks up the
// change and live-reloads Calendar automatically.
//
// Known limitation of the plain-text approach: if a *user* hand-edits
// one of these blocks and adds a blank line in the middle of it,
// `remove_event` stops at that blank line and leaves the rest behind
// rather than corrupting unrelated content — a hand-edited block just
// won't clean up perfectly, which is an acceptable trade-off for not
// needing a real TOML editor dependency to support removal.

fn calendar_toml_path() -> Result<std::path::PathBuf> {
    Ok(crate::config::config_dir()?.join("calendar.toml"))
}

fn extract_marker_comment(id: &str) -> String {
    format!("# docket:extract:{id}")
}

/// Whether an event previously added via `add_event(_, _, id)` is
/// still present. `Ok(false)` (not an error) if calendar.toml doesn't
/// exist yet — nothing has ever been added.
pub fn event_marker_present(id: &str) -> Result<bool> {
    let path = calendar_toml_path()?;
    if !path.exists() {
        return Ok(false);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    Ok(text.contains(&extract_marker_comment(id)))
}

/// True if `text` has a *non-commented* `kind = "local"` line —
/// i.e. an explicit `[[providers]]` entry activating the local
/// `[[events]]` source. Deliberately simple (a per-line substring
/// check, not a TOML parse) to match the rest of this module's
/// plain-text approach; a commented-out example (`# kind = "local"`)
/// correctly doesn't count.
fn has_local_provider(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim();
        !l.starts_with('#') && l.replace(' ', "") == "kind=\"local\""
    })
}

/// Registers a `[[providers]] kind = "local"` entry if one isn't
/// already present. **This is the load-bearing fix for `add_event`
/// actually showing up anywhere**: when `[[providers]]` is non-empty
/// (any external CalDAV/ICS source configured), docket's provider
/// wiring (`wiring::build_provider`) only builds *those* configured
/// providers — the `[[events]]` local source is silently dropped
/// unless a `local` entry explicitly opts it back in. Without this,
/// `add_event` would write a real event that never renders anywhere,
/// for anyone who has any other calendar source configured (i.e.
/// most users). No-op if a local provider is already registered
/// (including the common case of `[[providers]]` being empty, which
/// activates local events by itself — see `wiring::build_provider`).
fn ensure_local_provider_registered(path: &std::path::Path) -> Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    if existing.is_empty() || has_local_provider(&existing) {
        return Ok(());
    }
    let mut addition = String::new();
    if !existing.ends_with('\n') {
        addition.push('\n');
    }
    addition.push_str("\n[[providers]]\nkind = \"local\"\n");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    file.write_all(addition.as_bytes())
        .with_context(|| format!("append to {}", path.display()))
}

/// Append an all-day local event tagged with `id`. Idempotent — a
/// second call with the same `id` is a no-op, so callers don't need
/// to check `event_marker_present` first.
pub fn add_event(title: &str, date: &str, id: &str) -> Result<()> {
    let path = calendar_toml_path()?;
    if event_marker_present(id)? {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    ensure_local_provider_registered(&path)?;
    let needs_leading_newline = match std::fs::read_to_string(&path) {
        Ok(existing) => !existing.is_empty() && !existing.ends_with('\n'),
        Err(_) => false, // file doesn't exist yet — nothing to separate from
    };
    let escaped_title = title.replace('\\', "\\\\").replace('"', "\\\"");
    let mut block = String::new();
    if needs_leading_newline {
        block.push('\n');
    }
    block.push('\n');
    block.push_str(&extract_marker_comment(id));
    block.push('\n');
    block.push_str("[[events]]\n");
    block.push_str(&format!("title = \"{escaped_title}\"\n"));
    block.push_str(&format!("start = \"{date}\"\n"));
    block.push_str(&format!("end = \"{date}\"\n"));
    block.push_str("all_day = true\n");
    block.push_str("calendar = \"email\"\n");

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    file.write_all(block.as_bytes())
        .with_context(|| format!("append to {}", path.display()))?;
    Ok(())
}

/// Remove the event block tagged with `id`. `Ok(())` (not an error)
/// if it's already gone, or calendar.toml doesn't exist.
pub fn remove_event(id: &str) -> Result<()> {
    let path = calendar_toml_path()?;
    if !path.exists() {
        return Ok(());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let marker = extract_marker_comment(id);
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(start) = lines.iter().position(|l| l.trim() == marker) else {
        return Ok(());
    };
    let mut end = start + 1;
    while end < lines.len() && !lines[end].trim().is_empty() {
        end += 1;
    }
    // Swallow the blank-line separator `add_event` wrote before the
    // marker too, so repeated add/remove doesn't accumulate blank
    // lines — but only the one right after our block, never anything
    // before `start`.
    if end < lines.len() {
        end += 1;
    }
    let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
    kept.extend_from_slice(&lines[..start]);
    if end < lines.len() {
        kept.extend_from_slice(&lines[end..]);
    }
    std::fs::write(&path, kept.join("\n"))
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(start: &str, end: &str, all_day: bool) -> RawEvent {
        RawEvent {
            title: "x".into(),
            start: start.into(),
            end: end.into(),
            all_day,
            calendar: "default".into(),
            location: None,
        }
    }

    #[test]
    fn rfc3339_timed_event_parses_into_local_time() {
        let e = raw("2026-05-20T09:00:00Z", "2026-05-20T10:00:00Z", false)
            .parse()
            .unwrap();
        assert!(!e.all_day);
        assert!(e.end > e.start);
    }

    #[test]
    fn bare_date_treated_as_all_day_with_exclusive_end() {
        let e = raw("2026-05-20", "2026-05-20", false).parse().unwrap();
        assert!(e.all_day);
        assert_eq!(e.end - e.start, chrono::Duration::days(1));
    }

    #[tokio::test]
    async fn fetch_range_filters_and_sorts() {
        let file = LocalCalendarFile {
            events: vec![
                raw("2026-05-20T15:00:00Z", "2026-05-20T16:00:00Z", false),
                raw("2026-05-20T09:00:00Z", "2026-05-20T10:00:00Z", false),
                raw("2026-06-01T09:00:00Z", "2026-06-01T10:00:00Z", false),
            ],
        };
        let p = LocalCalendarProvider::from_file(file).unwrap();
        let start = Local.with_ymd_and_hms(2026, 5, 20, 0, 0, 0).unwrap();
        let end = Local.with_ymd_and_hms(2026, 5, 21, 0, 0, 0).unwrap();
        let got = p.fetch_range(start, end).await.unwrap();
        assert_eq!(got.len(), 2);
        assert!(got[0].start < got[1].start);
    }

    // ── Email-extract add/remove ────────────────────────────────────

    // Shared isolation helper — see its doc comment in
    // `widgets::test_support` for why this needs to be one process-
    // wide lock rather than a per-module one.
    use crate::widgets::test_support::IsolatedConfigHome;

    #[test]
    fn add_then_marker_present_then_remove_round_trips() {
        let _cfg = IsolatedConfigHome::new();
        assert!(!event_marker_present("id-1").unwrap());

        add_event("Budget review", "2026-09-03", "id-1").unwrap();
        assert!(event_marker_present("id-1").unwrap());

        let path = calendar_toml_path().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("title = \"Budget review\""));
        assert!(text.contains("start = \"2026-09-03\""));
        assert!(text.contains("all_day = true"));

        remove_event("id-1").unwrap();
        assert!(!event_marker_present("id-1").unwrap());
    }

    #[test]
    fn add_is_idempotent() {
        let _cfg = IsolatedConfigHome::new();
        add_event("Budget review", "2026-09-03", "id-2").unwrap();
        add_event("Budget review", "2026-09-03", "id-2").unwrap();
        let path = calendar_toml_path().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text.matches("title = \"Budget review\"").count(),
            1,
            "second add_event call must be a no-op"
        );
    }

    #[test]
    fn add_preserves_existing_file_content() {
        let _cfg = IsolatedConfigHome::new();
        let path = calendar_toml_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# a hand-written comment\ndefault_view = \"month\"\n").unwrap();

        add_event("Budget review", "2026-09-03", "id-3").unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# a hand-written comment"));
        assert!(text.contains("default_view = \"month\""));
        assert!(text.contains("title = \"Budget review\""));
    }

    #[test]
    fn add_registers_a_local_provider_when_the_file_only_has_external_ones() {
        let _cfg = IsolatedConfigHome::new();
        let path = calendar_toml_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Mirrors a real-world config: external providers only, no
        // local one — the exact shape that silently dropped
        // `[[events]]` (including anything add_event writes) before
        // this fix.
        std::fs::write(
            &path,
            "[[providers]]\nkind = \"ics\"\naccount = \"work\"\n",
        )
        .unwrap();
        assert!(!has_local_provider(&std::fs::read_to_string(&path).unwrap()));

        add_event("Budget review", "2026-09-03", "id-4").unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(has_local_provider(&text));
        assert!(text.contains("kind = \"ics\""), "existing provider must survive");
    }

    #[test]
    fn add_does_not_duplicate_an_existing_local_provider() {
        let _cfg = IsolatedConfigHome::new();
        let path = calendar_toml_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[[providers]]\nkind = \"local\"\n").unwrap();

        add_event("Budget review", "2026-09-03", "id-5").unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("kind = \"local\"").count(), 1);
    }

    #[test]
    fn has_local_provider_ignores_a_commented_out_example() {
        assert!(!has_local_provider("# kind = \"local\"\n"));
        assert!(has_local_provider("kind = \"local\"\n"));
        assert!(has_local_provider("  kind = \"local\"  \n"));
    }

    #[test]
    fn remove_of_never_added_event_is_a_noop() {
        let _cfg = IsolatedConfigHome::new();
        remove_event("never-added").unwrap(); // must not error
        assert!(!event_marker_present("never-added").unwrap());
    }

    #[test]
    fn remove_only_deletes_the_matching_block() {
        let _cfg = IsolatedConfigHome::new();
        add_event("First event", "2026-09-01", "id-a").unwrap();
        add_event("Second event", "2026-09-02", "id-b").unwrap();

        remove_event("id-a").unwrap();

        let path = calendar_toml_path().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("First event"));
        assert!(text.contains("Second event"));
        assert!(!event_marker_present("id-a").unwrap());
        assert!(event_marker_present("id-b").unwrap());
    }

    #[test]
    fn event_marker_present_is_false_when_calendar_toml_does_not_exist() {
        let _cfg = IsolatedConfigHome::new();
        assert!(!event_marker_present("anything").unwrap());
    }
}

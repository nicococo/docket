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
use chrono::{DateTime, Local, NaiveDate};
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

/// Re-read `config.toml` for the currently-configured `local_ics_path`.
/// Cheap and infrequent (only on an email extract add/remove), same
/// pattern as `email::extract_actions`' own `notes_dir` lookup — avoids
/// threading the already-loaded app `Config` through Email's key-
/// handling path just for this.
fn configured_ics_path() -> Result<PathBuf> {
    let configured = crate::config::load(None)
        .ok()
        .and_then(|cfg| cfg.calendar.local_ics_path);
    resolve_ics_path(configured.as_deref())
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

// ── Email-extract add/remove integration ───────────────────────────
//
// Lets the Email widget's "extract dates" AI popup action add/remove
// a local all-day event (see `email::extract_actions`), without a
// general cross-widget dependency. Two files are involved:
// - `config.toml`'s `[[calendar.providers]]` (via
//   `ensure_local_provider_registered`) — unrelated to event storage,
//   just makes sure the local source stays active when the user has
//   also configured an external CalDAV/ICS provider.
// - the local `.ics` file itself (`configured_ics_path()`) — where
//   the actual VEVENT blocks live.
//
// Both are edited as plain text, not parsed/rewritten wholesale:
// `add_event` inserts one new `BEGIN:VEVENT…END:VEVENT` block right
// before the file's `END:VCALENDAR` line (valid regardless of how
// many other events are already there), and `remove_event` only ever
// deletes a block carrying its own `X-DOCKET-EXTRACT-ID:<id>` marker
// property, bounded by that block's own `BEGIN:VEVENT`/`END:VEVENT` —
// it never touches any other event.
//
// Known limitation of the plain-text approach: a *user* hand-editing
// an extracted VEVENT to span multiple `BEGIN:VEVENT` blocks (not a
// realistic edit) could confuse `remove_event`'s block boundaries —
// an acceptable trade-off for not needing a full ICS writer/editor
// library to support removal.

fn extract_marker_property(id: &str) -> String {
    format!("X-DOCKET-EXTRACT-ID:{id}")
}

/// Whether an event previously added via `add_event(_, _, id)` is
/// still present. `Ok(false)` (not an error) if the `.ics` file
/// doesn't exist yet — nothing has ever been added.
pub fn event_marker_present(id: &str) -> Result<bool> {
    let path = configured_ics_path()?;
    if !path.exists() {
        return Ok(false);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    Ok(text.contains(&extract_marker_property(id)))
}

/// True if `text` has a *non-commented* `kind = "local"` line — i.e.
/// an explicit `[[calendar.providers]]` entry activating the local
/// `.ics` source. Deliberately simple (a per-line substring check,
/// not a TOML parse) to match the rest of this module's plain-text
/// approach; a commented-out example (`# kind = "local"`) correctly
/// doesn't count.
fn has_local_provider(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim();
        !l.starts_with('#') && l.replace(' ', "") == "kind=\"local\""
    })
}

/// Registers a `[[calendar.providers]] kind = "local"` entry in
/// `config.toml` if one isn't already present. **This is the load-
/// bearing fix for `add_event` actually showing up anywhere**: when
/// `[[calendar.providers]]` is non-empty (any external CalDAV/ICS
/// source configured), docket's provider wiring
/// (`wiring::build_provider`) only builds *those* configured
/// providers — the local `.ics` source is silently dropped unless a
/// `local` entry explicitly opts it back in. No-op if a local
/// provider is already registered (including the common case of
/// `[[calendar.providers]]` being empty, which activates local events
/// by itself — see `wiring::build_provider`).
fn ensure_local_provider_registered() -> Result<()> {
    let path = crate::config::config_path()?;
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing.is_empty() || has_local_provider(&existing) {
        return Ok(());
    }
    let mut addition = String::new();
    if !existing.ends_with('\n') {
        addition.push('\n');
    }
    addition.push_str("\n[[calendar.providers]]\nkind = \"local\"\n");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    file.write_all(addition.as_bytes())
        .with_context(|| format!("append to {}", path.display()))
}

/// Escape a text value per RFC 5545 §3.3.11: backslash, semicolon,
/// comma, and newline all need a leading backslash.
fn escape_ics_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

/// Wrap `vevent_block` (a complete `BEGIN:VEVENT…END:VEVENT\n` chunk)
/// in a `VCALENDAR` envelope if the file is new/empty, or insert it
/// just before the existing `END:VCALENDAR` otherwise — keeping
/// exactly one well-formed calendar in the file no matter how many
/// events have been added over time.
fn insert_vevent_block(path: &Path, vevent_block: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let body = if existing.trim().is_empty() {
        format!(
            "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//docket//local//EN\nCALSCALE:GREGORIAN\n{vevent_block}END:VCALENDAR\n"
        )
    } else if let Some(idx) = existing.rfind("END:VCALENDAR") {
        let mut out = String::with_capacity(existing.len() + vevent_block.len());
        out.push_str(&existing[..idx]);
        out.push_str(vevent_block);
        out.push_str(&existing[idx..]);
        out
    } else {
        // Malformed/missing envelope — wrap what's there defensively
        // rather than losing it.
        format!(
            "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//docket//local//EN\nCALSCALE:GREGORIAN\n{existing}{vevent_block}END:VCALENDAR\n"
        )
    };
    std::fs::write(path, body).with_context(|| format!("write {}", path.display()))
}

/// Append an all-day local event tagged with `id`. Idempotent — a
/// second call with the same `id` is a no-op, so callers don't need
/// to check `event_marker_present` first.
pub fn add_event(title: &str, date: &str, id: &str) -> Result<()> {
    if event_marker_present(id)? {
        return Ok(());
    }
    ensure_local_provider_registered()?;

    let start = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .with_context(|| format!("invalid date {date:?}"))?;
    let end_exclusive = start
        .succ_opt()
        .context("date overflow computing exclusive end")?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let block = format!(
        "BEGIN:VEVENT\nUID:docket-extract-{id}@docket\nDTSTAMP:{stamp}\nDTSTART;VALUE=DATE:{}\nDTEND;VALUE=DATE:{}\nSUMMARY:{}\nCATEGORIES:email\n{}\nEND:VEVENT\n",
        start.format("%Y%m%d"),
        end_exclusive.format("%Y%m%d"),
        escape_ics_text(title),
        extract_marker_property(id),
    );
    insert_vevent_block(&configured_ics_path()?, &block)
}

/// Remove the event block tagged with `id`. `Ok(())` (not an error)
/// if it's already gone, or the `.ics` file doesn't exist.
pub fn remove_event(id: &str) -> Result<()> {
    let path = configured_ics_path()?;
    if !path.exists() {
        return Ok(());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let marker = extract_marker_property(id);
    let lines: Vec<&str> = text.lines().collect();
    let Some(marker_idx) = lines.iter().position(|l| l.trim() == marker) else {
        return Ok(());
    };
    let mut start = marker_idx;
    while start > 0 && lines[start].trim() != "BEGIN:VEVENT" {
        start -= 1;
    }
    let mut end = marker_idx;
    while end < lines.len() && lines[end].trim() != "END:VEVENT" {
        end += 1;
    }
    let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
    kept.extend_from_slice(&lines[..start]);
    if end + 1 < lines.len() {
        kept.extend_from_slice(&lines[end + 1..]);
    }
    let mut joined = kept.join("\n");
    joined.push('\n');
    std::fs::write(&path, joined).with_context(|| format!("write {}", path.display()))?;
    Ok(())
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

        let path = configured_ics_path().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("SUMMARY:Budget review"));
        assert!(text.contains("DTSTART;VALUE=DATE:20260903"));
        assert!(text.contains("DTEND;VALUE=DATE:20260904"));
        assert!(text.contains("BEGIN:VCALENDAR"));
        assert!(text.contains("END:VCALENDAR"));

        remove_event("id-1").unwrap();
        assert!(!event_marker_present("id-1").unwrap());
    }

    #[test]
    fn add_is_idempotent() {
        let _cfg = IsolatedConfigHome::new();
        add_event("Budget review", "2026-09-03", "id-2").unwrap();
        add_event("Budget review", "2026-09-03", "id-2").unwrap();
        let path = configured_ics_path().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text.matches("SUMMARY:Budget review").count(),
            1,
            "second add_event call must be a no-op"
        );
    }

    #[test]
    fn add_preserves_existing_events_in_the_file() {
        let _cfg = IsolatedConfigHome::new();
        add_event("First event", "2026-09-01", "id-a").unwrap();
        add_event("Second event", "2026-09-02", "id-b").unwrap();

        let path = configured_ics_path().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("SUMMARY:First event"));
        assert!(text.contains("SUMMARY:Second event"));
        // Exactly one calendar envelope, not one per event.
        assert_eq!(text.matches("BEGIN:VCALENDAR").count(), 1);
        assert_eq!(text.matches("END:VCALENDAR").count(), 1);
    }

    #[test]
    fn add_registers_a_local_provider_when_config_only_has_external_ones() {
        let _cfg = IsolatedConfigHome::new();
        let config_path = crate::config::config_path().unwrap();
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        // Mirrors a real-world config: external providers only, no
        // local one — the exact shape that would otherwise silently
        // drop the local .ics source entirely.
        std::fs::write(
            &config_path,
            "[[calendar.providers]]\nkind = \"ics\"\naccount = \"work\"\n",
        )
        .unwrap();
        assert!(!has_local_provider(
            &std::fs::read_to_string(&config_path).unwrap()
        ));

        add_event("Budget review", "2026-09-03", "id-4").unwrap();

        let text = std::fs::read_to_string(&config_path).unwrap();
        assert!(has_local_provider(&text));
        assert!(text.contains("kind = \"ics\""), "existing provider must survive");
    }

    #[test]
    fn add_does_not_duplicate_an_existing_local_provider() {
        let _cfg = IsolatedConfigHome::new();
        let config_path = crate::config::config_path().unwrap();
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(&config_path, "[[calendar.providers]]\nkind = \"local\"\n").unwrap();

        add_event("Budget review", "2026-09-03", "id-5").unwrap();

        let text = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(text.matches("kind = \"local\"").count(), 1);
    }

    #[test]
    fn has_local_provider_ignores_a_commented_out_example() {
        assert!(!has_local_provider("# kind = \"local\"\n"));
        assert!(has_local_provider("kind = \"local\"\n"));
        assert!(has_local_provider("  kind = \"local\"  \n"));
    }

    /// Regression-style test: round-trips through the REAL ICS parser
    /// (`LocalCalendarProvider::from_ics_file`), not just a raw-string
    /// assertion on the file `add_event` itself wrote — catches a
    /// future malformed-VEVENT regression that string-contains checks
    /// alone wouldn't.
    #[test]
    fn add_event_is_actually_parseable_by_the_real_local_provider() {
        let _cfg = IsolatedConfigHome::new();
        add_event("Budget review", "2026-09-03", "id-roundtrip").unwrap();

        let path = configured_ics_path().unwrap();
        let provider = LocalCalendarProvider::from_ics_file(&path).unwrap();
        assert!(
            provider
                .events
                .iter()
                .any(|e| e.title == "Budget review" && e.all_day),
            "the extracted event must be visible through the same ICS parser \
             the Calendar widget actually uses, not just as raw text in the file"
        );
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

        let path = configured_ics_path().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("First event"));
        assert!(text.contains("Second event"));
        assert!(!event_marker_present("id-a").unwrap());
        assert!(event_marker_present("id-b").unwrap());
        // File must still be a well-formed single calendar.
        assert_eq!(text.matches("BEGIN:VCALENDAR").count(), 1);
        assert_eq!(text.matches("END:VCALENDAR").count(), 1);
    }

    #[test]
    fn event_marker_present_is_false_when_ics_file_does_not_exist() {
        let _cfg = IsolatedConfigHome::new();
        assert!(!event_marker_present("anything").unwrap());
    }
}

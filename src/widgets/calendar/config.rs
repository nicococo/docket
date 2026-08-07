// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! Configuration schema for the calendar widget — TOML on-disk shape
//! and defaults. No render/state code here; this is the
//! data-and-schema layer the rest of the widget reads from.

use std::collections::HashMap;

use chrono::Weekday;
use serde::{Deserialize, Serialize};

use super::local;
use crate::theme::ColorScheme;
use crate::ui::big_digits;

pub(super) const VIEW_TABS: &[(CalendarView, &str)] = &[
    (CalendarView::Day, "day"),
    (CalendarView::Week, "week"),
    (CalendarView::Month, "month"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CalendarView {
    #[default]
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    #[default]
    Local,
    Google,
    #[serde(alias = "apple", alias = "icloud")]
    Caldav,
    #[serde(alias = "microsoft", alias = "ms365")]
    Outlook,
    /// Plain `.ics` HTTP(S) feed — no CalDAV discovery, no OAuth. See
    /// `super::ics`.
    #[serde(alias = "ical", alias = "webcal")]
    Ics,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CalendarConfig {
    #[serde(default)]
    pub default_view: CalendarView,

    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,

    /// Calendar sources. Empty = local-only (use `[[events]]` below).
    #[serde(default)]
    pub providers: Vec<ProviderEntry>,

    /// Fallback URLs for any `caldav` entry without explicit `calendar_ids`.
    #[serde(default)]
    pub caldav: CalDavConfig,

    /// Events for the built-in local provider.
    #[serde(default)]
    pub events: Vec<local::RawEvent>,

    /// ANSI palette cycled across calendars in `[[providers]]` order. Names
    /// like `red`, `light_blue`. Wraps when more calendars than colors.
    #[serde(default)]
    pub color_palette: Vec<String>,

    /// Per-calendar overrides keyed by `"<source>:<calendar_id>"`
    /// (e.g. `"google:primary"`). Wins over the palette sequence.
    #[serde(default)]
    pub calendar_colors: HashMap<String, String>,

    /// Big-digit gradient for the day-of-month numeral in Day view.
    /// `g` cycles. Only applies to today — anchor/preview days stay solid.
    #[serde(default)]
    pub gradient: big_digits::Gradient,

    /// Per-widget overrides layered on the app theme. Distinct from
    /// `calendar_colors`, which colors per-provider event blocks.
    #[serde(default)]
    pub colors: ColorScheme,

    /// `Shift+<letter>` focus shortcuts; falls back to `['c', 'd', 'a', 'l', 'e', 'n', 'r']`.
    #[serde(default)]
    pub shortcuts: Vec<char>,

    /// Which weekday starts the week in Week + Month views. Defaults to
    /// Sunday (US convention); ISO/Europe users typically set
    /// `first_day_of_week = "monday"`. Any chrono-recognized lowercase
    /// weekday name works (sunday/monday/tuesday/...). Invalid values
    /// fall back to Sunday with a `serde` parse error logged.
    #[serde(default)]
    pub first_day_of_week: FirstDayOfWeek,
}

/// Configurable first-day-of-week. Defaults to Sunday. Serialized as
/// a lowercase weekday name (`"sunday"`, `"monday"`, …) so the TOML
/// reads naturally.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FirstDayOfWeek {
    #[default]
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

impl FirstDayOfWeek {
    pub fn as_weekday(self) -> Weekday {
        match self {
            FirstDayOfWeek::Sunday => Weekday::Sun,
            FirstDayOfWeek::Monday => Weekday::Mon,
            FirstDayOfWeek::Tuesday => Weekday::Tue,
            FirstDayOfWeek::Wednesday => Weekday::Wed,
            FirstDayOfWeek::Thursday => Weekday::Thu,
            FirstDayOfWeek::Friday => Weekday::Fri,
            FirstDayOfWeek::Saturday => Weekday::Sat,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderEntry {
    pub kind: ProviderKind,
    /// Account label for same-provider multi-account (e.g. a work Outlook
    /// alongside a personal one). Omitted ⇒ the `"default"` account. The
    /// label names which `…_oauth_token.<account>.toml` to use and, when
    /// non-default, becomes this entry's `source` so colors don't collide.
    /// Google and Outlook are OAuth-account-aware; `ics` reuses the same
    /// field as the feed label matched against `credentials/ics.toml`'s
    /// `[[feeds]]` entries (omitted ⇒ the feed labeled `"default"`).
    #[serde(default)]
    pub account: Option<String>,
    /// Google IDs, Outlook IDs, or CalDAV URLs. Empty = the provider's default
    /// (Google `"primary"`, Outlook default, every CalDAV calendar).
    #[serde(default)]
    pub calendar_ids: Vec<String>,
}

impl ProviderEntry {
    /// Token-storage account label — the explicit `account`, or `"default"`.
    pub(super) fn account_label(&self) -> &str {
        self.account.as_deref().unwrap_or(crate::auth::DEFAULT_ACCOUNT)
    }

    /// Identity used for the cell title + color keys. The default account
    /// reads as the provider kind (`"outlook"`) so existing single-account
    /// configs and `calendar_colors` keys are unaffected; a named account is
    /// provider-namespaced as `kind/account` (`"outlook/work"`) so it stays
    /// grouped under its provider and never collides with a same-label
    /// account of a different kind. `/` (not `:`) because `:` already
    /// separates source from calendar in `calendar_colors` keys.
    pub(super) fn source_label(&self) -> String {
        let kind = super::colors::provider_kind_label(self.kind);
        match &self.account {
            Some(a) if a != crate::auth::DEFAULT_ACCOUNT => format!("{kind}/{a}"),
            _ => kind.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CalDavConfig {
    /// Explicit calendar URLs. Empty = walk the CalDAV principal chain
    /// (current-user-principal → calendar-home-set → calendars) to discover.
    #[serde(default)]
    pub calendars: Vec<String>,
}

fn default_poll_interval() -> u64 {
    60
}

impl Default for CalendarConfig {
    fn default() -> Self {
        Self {
            default_view: CalendarView::default(),
            poll_interval_secs: default_poll_interval(),
            providers: Vec::new(),
            caldav: CalDavConfig::default(),
            events: Vec::new(),
            color_palette: Vec::new(),
            calendar_colors: HashMap::new(),
            gradient: big_digits::Gradient::default(),
            colors: ColorScheme::default(),
            shortcuts: Vec::new(),
            first_day_of_week: FirstDayOfWeek::default(),
        }
    }
}

pub const KIND: &str = "calendar";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_label_defaults_to_kind_but_names_account() {
        let default = ProviderEntry {
            kind: ProviderKind::Outlook,
            account: None,
            calendar_ids: vec![],
        };
        assert_eq!(default.source_label(), "outlook");
        assert_eq!(default.account_label(), "default");

        // An explicit account = "default" still colors as the kind.
        let explicit_default = ProviderEntry {
            kind: ProviderKind::Outlook,
            account: Some("default".into()),
            calendar_ids: vec![],
        };
        assert_eq!(explicit_default.source_label(), "outlook");

        // A named account is provider-namespaced for its source/color key,
        // but its *token* account label stays the bare string.
        let work = ProviderEntry {
            kind: ProviderKind::Outlook,
            account: Some("work".into()),
            calendar_ids: vec![],
        };
        assert_eq!(work.source_label(), "outlook/work");
        assert_eq!(work.account_label(), "work");

        // Same label under a different provider gets a distinct source.
        let g_work = ProviderEntry {
            kind: ProviderKind::Google,
            account: Some("work".into()),
            calendar_ids: vec![],
        };
        assert_eq!(g_work.source_label(), "google/work");
        assert_ne!(g_work.source_label(), work.source_label());
    }
}

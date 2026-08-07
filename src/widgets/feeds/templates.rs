// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! Starter templates for the feeds widget. When an instance has no
//! `[[feeds]]` blocks configured (a brand-new `feeds.toml`), the
//! widget seeds itself from the WSJ template so there's something to
//! render immediately — see the fallback in `feeds::build`.
//!
//! Each template TOML lives at `src/widgets/feeds/templates/<id>.toml`
//! and is embedded into the binary at build time via `include_str!`,
//! parsed into [`Template`] values on demand.
//!
//! Adding a new built-in template: drop a new TOML in
//! `src/widgets/feeds/templates/` and add a matching `include_str!`
//! line to [`BUILTIN_TEMPLATES`] below.

use serde::Deserialize;

/// One starter template parsed from a TOML file.
#[derive(Debug, Clone, Deserialize)]
pub struct Template {
    /// Stable id used to look the template up via [`by_id`]. Lowercase
    /// ASCII, no whitespace.
    pub id: String,

    /// Human-readable name. Reserved for a future flow that scaffolds a
    /// fresh instance TOML from a template (would seed `display_name =
    /// "..."`, which the runtime uses for the title bar, dashboard
    /// label, and LLM summarizer prompt).
    #[allow(dead_code)]
    pub display_name: String,

    /// Preferred `Shift+<letter>` shortcut letters. Reserved for the
    /// same future scaffolding flow as `display_name`.
    #[allow(dead_code)]
    pub default_shortcut_prefs: Vec<char>,

    /// Per-instance command aliases (`:<alias>`, `:<alias>-summary`,
    /// `:<alias>-refresh`). Reserved for the same future scaffolding
    /// flow as `display_name`.
    #[allow(dead_code)]
    #[serde(default)]
    pub default_commands: Vec<String>,

    /// Every topical feed this source publishes. `default = true`
    /// entries are written as live `[[feeds]]` blocks; the rest are
    /// written commented-out so the user can uncomment to enable.
    #[serde(default)]
    pub feeds: Vec<TemplateFeed>,
}

/// One catalogue entry inside a [`Template`].
#[derive(Debug, Clone, Deserialize)]
pub struct TemplateFeed {
    pub topic: String,
    pub url: String,
    /// Whether this is one of the feeds seeded into a fresh instance
    /// when no `[[feeds]]` blocks are configured yet.
    #[serde(default)]
    pub default: bool,
}

/// Every compiled-in template. Each entry is `(id, raw_toml)`; parsed
/// on demand rather than at static-init time so a malformed template
/// only fails the lookup that needed it, not the whole binary.
const BUILTIN_TEMPLATES: &[(&str, &str)] = &[
    ("wsj", include_str!("templates/wsj.toml")),
    (
        "marketwatch",
        include_str!("templates/marketwatch.toml"),
    ),
];

/// Parse every built-in template. Panics on a malformed embedded TOML
/// — those failures are programmer errors caught by the test below,
/// not user-supplied bad data.
pub fn all() -> Vec<Template> {
    BUILTIN_TEMPLATES
        .iter()
        .map(|(id, raw)| {
            toml::from_str::<Template>(raw)
                .unwrap_or_else(|err| panic!("templates/{id}.toml: parse failed: {err}"))
        })
        .collect()
}

/// Look up a single template by id (case-insensitive). Returns
/// `None` for unknown ids so callers can fall back to WSJ rather
/// than panic on an unrecognized id.
pub fn by_id(id: &str) -> Option<Template> {
    all().into_iter().find(|t| t.id.eq_ignore_ascii_case(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_template_parses() {
        let parsed = all();
        assert_eq!(
            parsed.len(),
            BUILTIN_TEMPLATES.len(),
            "every embedded template should parse"
        );
    }

    #[test]
    fn template_ids_are_lowercase_unique_and_non_empty() {
        let mut seen = std::collections::HashSet::new();
        for t in all() {
            assert!(!t.id.is_empty(), "template id must not be empty");
            assert_eq!(
                t.id,
                t.id.to_ascii_lowercase(),
                "template id must be lowercase: {}",
                t.id
            );
            assert!(seen.insert(t.id.clone()), "duplicate template id: {}", t.id);
        }
    }

    #[test]
    fn template_id_matches_file_slug() {
        for (slug, _) in BUILTIN_TEMPLATES {
            let t = by_id(slug).unwrap_or_else(|| {
                panic!("by_id({slug}) returned None — template id doesn't match filename")
            });
            assert_eq!(
                t.id, *slug,
                "templates/{slug}.toml declares id = {:?}; should match its filename",
                t.id
            );
        }
    }

    #[test]
    fn templates_have_at_least_one_default_feed() {
        for t in all() {
            assert!(
                t.feeds.iter().any(|f| f.default),
                "template {} should have at least one default feed",
                t.id
            );
        }
    }

    #[test]
    fn no_duplicate_topics_within_a_template() {
        for t in all() {
            let mut seen = std::collections::HashSet::new();
            for feed in &t.feeds {
                assert!(
                    seen.insert(feed.topic.clone()),
                    "template {}: duplicate topic {:?}",
                    t.id,
                    feed.topic
                );
            }
        }
    }

    #[test]
    fn by_id_is_case_insensitive() {
        assert!(by_id("wsj").is_some());
        assert!(by_id("WSJ").is_some());
        assert!(by_id("Wsj").is_some());
        assert!(by_id("marketwatch").is_some());
        assert!(by_id("MarketWatch").is_some());
        assert!(by_id("definitely-not-a-source").is_none());
    }
}

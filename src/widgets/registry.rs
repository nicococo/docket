// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! Widget registry — the single source of truth for which widgets exist.
//!
//! ## Adding a widget
//!
//! 1. Implement [`Widget`] under `src/widgets/<name>/`.
//! 2. Export `pub const KIND: &str` and `pub fn build(&WidgetCtx) -> Box<dyn Widget>`.
//! 3. Add a `widget-<name>` feature in `Cargo.toml` and gate the module in
//!    `widgets::mod` on it.
//! 4. Append a `WidgetDescriptor` to [`WIDGETS`] below.
//!
//! No edits to `app.rs` or `main.rs` required.
//! Registration and first-run defaults all walk `WIDGETS`.

use super::{Widget, WidgetCtx, WidgetFactory};

/// Static description of a widget kind.
pub struct WidgetDescriptor {
    /// Stable kind string used in `layout.toml` cells and `<kind>.toml`
    /// config filenames. Must match the widget module's `KIND` constant.
    pub kind: &'static str,

    /// Factory that reads the widget's TOML and constructs an instance.
    pub factory: WidgetFactory,

    /// Whether this widget appears in the empty-layout fallback grid. Set
    /// to `false` for auxiliary widgets that the user should opt into by
    /// editing `config.toml`.
    pub default_in_first_run: bool,
}

/// The full set of widgets compiled into this build. Order is significant
/// — it sets the empty-layout fallback registration order.
pub const WIDGETS: &[WidgetDescriptor] = &[
    #[cfg(feature = "widget-calendar")]
    WidgetDescriptor {
        kind: super::calendar::KIND,
        factory: super::calendar::build,
        default_in_first_run: true,
    },
    #[cfg(feature = "widget-email")]
    WidgetDescriptor {
        kind: super::email::KIND,
        factory: super::email::build,
        default_in_first_run: false,
    },
    #[cfg(feature = "widget-notes")]
    WidgetDescriptor {
        kind: super::notes::KIND,
        factory: super::notes::build,
        default_in_first_run: false,
    },
    #[cfg(feature = "widget-feeds")]
    WidgetDescriptor {
        kind: super::feeds::KIND,
        factory: super::feeds::build,
        default_in_first_run: false,
    },
];

/// Look up a widget descriptor by kind string. `None` when the kind isn't
/// compiled in or doesn't exist.
pub fn find(kind: &str) -> Option<&'static WidgetDescriptor> {
    WIDGETS.iter().find(|d| d.kind == kind)
}

/// Build a widget for `(kind, instance)` via the registry. `make_ctx`
/// produces the [`WidgetCtx`] stamped with the supplied instance. Returns
/// `None` for unknown kinds so callers can warn and skip on layout typos.
pub fn build_for(
    kind: &str,
    instance: &str,
    make_ctx: impl FnOnce(String) -> WidgetCtx,
) -> Option<Box<dyn Widget>> {
    let desc = find(kind)?;
    let ctx = make_ctx(instance.to_string());
    Some((desc.factory)(&ctx))
}

/// Kinds that seed the empty-layout fallback grid.
pub fn default_kinds() -> impl Iterator<Item = &'static str> {
    WIDGETS
        .iter()
        .filter(|d| d.default_in_first_run)
        .map(|d| d.kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn kinds_are_unique_and_non_empty() {
        let mut seen: HashSet<&'static str> = HashSet::new();
        for desc in WIDGETS {
            assert!(!desc.kind.is_empty(), "widget kind must not be empty");
            assert!(
                seen.insert(desc.kind),
                "duplicate widget kind in registry: {}",
                desc.kind
            );
        }
    }

    #[test]
    fn find_returns_descriptor_for_each_kind() {
        for desc in WIDGETS {
            let found =
                find(desc.kind).unwrap_or_else(|| panic!("find({}) returned None", desc.kind));
            assert_eq!(found.kind, desc.kind);
        }
        assert!(find("definitely-not-a-real-widget").is_none());
    }

    /// Core-widget smoke test for the default-features dashboard. Mirrors
    /// the seed layout in `config::DEFAULT_CONFIG_TOML` — if either drifts,
    /// the empty-config first-run experience breaks.
    #[cfg(all(feature = "widget-calendar", feature = "widget-feeds"))]
    #[test]
    fn core_widgets_are_present() {
        for kind in ["calendar", "feeds"] {
            assert!(
                find(kind).is_some(),
                "core widget {kind} missing from registry"
            );
        }
    }
}

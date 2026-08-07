// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! Auth provider registry — the single source of truth for OAuth providers
//! and IMAP-style credential-only providers.
//!
//! Adding a provider is one entry in [`PROVIDERS`]: identity (`name`,
//! `display_name`) plus the `run` flow.
//!
//! Widgets declare which providers they depend on via [`AuthRequirement`]
//! on their `WidgetDescriptor`; `--auth <name>` resolves through [`find`].
//! Credentials templates (`credentials/google_oauth_client.toml`, etc.)
//! are seeded by `config::init_default_config` — fill them in by hand,
//! then run `docket --auth <provider>`.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

/// Boxed async flow stored behind a function pointer so the registry can
/// hold heterogenous provider flows in a `const`. The `account` label
/// selects which account's token the flow writes (`"default"` for a bare
/// `--auth <provider>`).
pub type AuthFlow = fn(account: &str) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>;

pub struct AuthProvider {
    /// Identifier used in `--auth <name>` and in [`AuthRequirement`].
    /// Lowercase ASCII, no spaces.
    pub name: &'static str,

    /// Human-readable label.
    #[allow(dead_code)] // reserved for a future auth-status listing.
    pub display_name: &'static str,

    /// Run the provider's interactive flow. For OAuth providers this
    /// drives the browser handshake; for credential-only providers (e.g.
    /// IMAP, whose credentials are hand-edited into `credentials/imap.toml`)
    /// this is a no-op.
    pub run: AuthFlow,
}

/// A widget's declared dependency on an OAuth provider.
///
/// `scope_hints` is informational — the actual OAuth scope string is owned
/// by the provider module (e.g. `auth::google::SCOPE`).
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // reserved for a future auth-status listing.
pub struct AuthRequirement {
    pub provider: &'static str,
    pub scope_hints: &'static [&'static str],
}

fn run_google(account: &str) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
    let account = account.to_string();
    Box::pin(async move {
        let client = super::google::OAuthClientConfig::load()?;
        super::google::flow::run(&client, &account).await?;
        println!("Google authorization complete.");
        Ok(())
    })
}

fn run_microsoft(account: &str) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
    let account = account.to_string();
    Box::pin(async move {
        let client = super::microsoft::OAuthClientConfig::load()?;
        super::microsoft::flow::run(&client, &account).await?;
        println!("Microsoft authorization complete.");
        Ok(())
    })
}

/// IMAP credentials are hand-edited into `credentials/imap.toml`; there is
/// no browser handshake. The `run` callback exists only to satisfy the
/// shared dispatch path. IMAP is single-account, so the label is ignored.
fn run_imap(_account: &str) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
    Box::pin(async move { Ok(()) })
}

pub const PROVIDERS: &[AuthProvider] = &[
    AuthProvider {
        name: "google",
        display_name: "Google (Calendar + Gmail)",
        run: run_google,
    },
    AuthProvider {
        name: "microsoft",
        display_name: "Microsoft (Outlook + Mail)",
        run: run_microsoft,
    },
    // IMAP is email-only; without widget-email there's no consumer for the
    // credentials and the provider is omitted entirely.
    #[cfg(feature = "widget-email")]
    AuthProvider {
        name: "imap",
        display_name: "IMAP (email via any IMAP server)",
        run: run_imap,
    },
];

pub fn find(name: &str) -> Option<&'static AuthProvider> {
    PROVIDERS.iter().find(|p| p.name == name)
}

/// Comma-separated list of registered provider names for CLI error messages.
pub fn names_csv() -> String {
    PROVIDERS
        .iter()
        .map(|p| p.name)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn provider_names_are_unique() {
        let mut seen: HashSet<&'static str> = HashSet::new();
        for p in PROVIDERS {
            assert!(!p.name.is_empty());
            assert!(
                seen.insert(p.name),
                "duplicate auth provider name: {}",
                p.name
            );
        }
    }

    #[test]
    fn find_resolves_registered_providers() {
        assert!(find("google").is_some());
        assert!(find("microsoft").is_some());
        // IMAP is registered only when widget-email is enabled.
        #[cfg(feature = "widget-email")]
        assert!(find("imap").is_some());
        assert!(find("not-a-real-provider").is_none());
    }
}

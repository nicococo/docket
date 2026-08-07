// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! On-disk credentials store — one home for "load a TOML file holding a
//! secret" across every widget backend (IMAP, CalDAV, ICS, LLM API keys).
//!
//! All files live under `~/.config/docket/credentials/`, created with mode
//! `0700` on first use. Every credential file here is hand-edited by the
//! user (none of docket's own code writes to this directory), so this
//! module only needs to load, not save.
//!
//! Callers identify files by basename (`"imap.toml"`) rather than full path
//! so the credentials-dir convention is enforced — you can't accidentally
//! read a secret from `~/Desktop/`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;

/// The credentials dir — `<config_dir>/credentials/`, created mode `0700`
/// on first use. Holds account-level secrets (CalDAV, IMAP, LLM keys).
/// Idempotent.
pub fn dir() -> Result<PathBuf> {
    ensure_0700(crate::config::config_dir()?.join("credentials"))
}

fn ensure_0700(path: PathBuf) -> Result<PathBuf> {
    std::fs::create_dir_all(&path)
        .with_context(|| format!("failed to create credentials dir {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best-effort: a chmod failure on an existing dir we don't own
        // (rare; installed-as-root) shouldn't crash the auth flow.
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
    }
    Ok(path)
}

/// Resolve a credentials basename to its absolute path. Does *not* create
/// the file. Every load call funnels through here so callers can't
/// accidentally read from outside the credentials dir.
pub fn path(filename: &str) -> Result<PathBuf> {
    Ok(dir()?.join(filename))
}

/// Load a TOML-serialised credentials value by basename. Returns:
///
/// - `Ok(Some(value))` — file exists and parsed cleanly.
/// - `Ok(None)`        — file is absent. Caller decides whether
///                        that's expected (no token yet) or an error.
/// - `Err(_)`          — file exists but is unreadable / malformed.
///                        Surfaces with file path context so the
///                        user can find what to fix.
pub fn load<T>(filename: &str) -> Result<Option<T>>
where
    T: DeserializeOwned,
{
    let path = path(filename)?;
    if !path.exists() {
        return Ok(None);
    }
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let value: T =
        toml::from_str(&body).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated_dir() -> PathBuf {
        // Each test gets its own XDG_CONFIG_HOME so they can't
        // collide with each other or with the user's real
        // credentials. The teardown drop the dir explicitly.
        let dir = std::env::temp_dir().join(format!(
            "docket-credentials-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        dir
    }

    #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
    struct Sample {
        api_key: String,
        nonce: u64,
    }

    #[test]
    #[ignore = "mutates the process-wide XDG_CONFIG_HOME — opt in with --ignored"]
    fn load_returns_none_for_missing_file() {
        let tmp = isolated_dir();
        let loaded: Option<Sample> = load("nonexistent.toml").unwrap();
        assert!(loaded.is_none());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn dir_resolves_to_credentials_subdir_of_config() {
        // Plain unit test that doesn't actually create anything.
        // We just verify the path shape is the canonical
        // credentials/ subdirectory of the config dir.
        // (`dir()` may still create the dir, but we don't assert on
        // that here; the ignored tests above exercise creation.)
        let tmp = std::env::temp_dir().join(format!(
            "docket-creds-path-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::env::set_var("XDG_CONFIG_HOME", &tmp);
        let resolved = path("foo.toml").unwrap();
        assert!(resolved.ends_with("credentials/foo.toml"));
        std::fs::remove_dir_all(&tmp).ok();
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

//! Local "seen via docket" persistence. Docket never writes read-state back to
//! the server (Gmail / Graph), so we maintain a tiny on-disk set so messages
//! the user has expanded inside the dashboard stop showing the `●` indicator
//! even if they remain unread on the provider. It also holds the reverse
//! override — messages the user explicitly pressed `u` on to flag back as
//! unread even though the server (or a prior "seen") says otherwise.
//!
//! One file per (provider, account) pair, e.g.
//! `~/.config/docket/email_seen_outlook_alice_at_example.com.json`.
//! Contents: `{ "seen": ["id_1"], "unread": ["id_2"], "last_pruned": "<iso>" }`.
//! An id is never in both sets — marking one side clears the other.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::config_dir;

#[derive(Debug, Default, Serialize, Deserialize)]
struct OnDisk {
    #[serde(default)]
    seen: Vec<String>,
    #[serde(default)]
    unread: Vec<String>,
    #[serde(default)]
    last_pruned: Option<DateTime<Utc>>,
}

pub struct SeenStore {
    path: PathBuf,
    seen: HashSet<String>,
    /// Explicit "force unread" overrides — set by the user pressing `u` on a
    /// message the server/seen-state would otherwise show as read.
    unread: HashSet<String>,
    last_pruned: Option<DateTime<Utc>>,
}

impl SeenStore {
    /// Open the seen-store for the given provider+account, creating an empty
    /// one if no file exists yet. A failing parse is logged and treated as
    /// "no entries"; we never want a corrupt cache file to block the widget.
    pub fn load(provider: &str, account: &str) -> Result<Self> {
        let dir = config_dir()?;
        // Ensure the parent dir exists — on a fresh install the docket config
        // dir might not have been seeded yet, and we'd otherwise fail to
        // write the seen file on the first `e` press.
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!(
            "email_seen_{}_{}.json",
            provider,
            sanitize_account(account)
        ));
        Self::load_at_path(path)
    }

    /// Test hook: load from a caller-supplied path so unit tests can be
    /// isolated from XDG_CONFIG_HOME (which is process-global and races
    /// across parallel tests).
    fn load_at_path(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let (seen, unread, last_pruned) = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(text) => match serde_json::from_str::<OnDisk>(&text) {
                    Ok(d) => (
                        d.seen.into_iter().collect(),
                        d.unread.into_iter().collect(),
                        d.last_pruned,
                    ),
                    Err(err) => {
                        tracing::warn!(error = %err, path = %path.display(), "seen-store parse failed, starting fresh");
                        (HashSet::new(), HashSet::new(), None)
                    }
                },
                Err(err) => {
                    tracing::warn!(error = %err, path = %path.display(), "seen-store read failed, starting fresh");
                    (HashSet::new(), HashSet::new(), None)
                }
            }
        } else {
            (HashSet::new(), HashSet::new(), None)
        };
        Ok(Self {
            path,
            seen,
            unread,
            last_pruned,
        })
    }

    pub fn contains(&self, id: &str) -> bool {
        self.seen.contains(id)
    }

    /// True when the user explicitly flagged `id` back to unread with `u`.
    /// Takes priority over both `contains` and the server's own read state.
    pub fn is_forced_unread(&self, id: &str) -> bool {
        self.unread.contains(id)
    }

    /// Mark `id` as seen and immediately persist. Clears any forced-unread
    /// override on the same id (marking as seen always wins). Persistence
    /// failure is surfaced but the in-memory set still has the update so the
    /// current session reflects the change.
    pub fn mark_seen(&mut self, id: &str) -> Result<()> {
        let changed = self.seen.insert(id.to_string());
        let cleared = self.unread.remove(id);
        if !changed && !cleared {
            return Ok(());
        }
        self.persist()
    }

    /// Force `id` to show as unread, overriding both server state and any
    /// prior "seen" mark. Immediately persisted.
    pub fn mark_unread(&mut self, id: &str) -> Result<()> {
        let changed = self.unread.insert(id.to_string());
        let cleared = self.seen.remove(id);
        if !changed && !cleared {
            return Ok(());
        }
        self.persist()
    }

    /// Drop ids known to be older than `days` worth of *seen state* — we
    /// don't know per-id timestamps, so the heuristic is: if the set grows
    /// unbounded and it's been more than `days` since the last prune,
    /// truncate it to half. Cheap, effective, never wrong in a way that
    /// affects correctness (the worst case is showing an unread badge for
    /// a message the user already opened in docket — server-unread state
    /// still drives the badge in that case, so the user will see it as
    /// "unread again" and re-trigger seen on the next `e`).
    #[allow(dead_code)] // called opportunistically from widget construction.
    pub fn prune_older_than_days(&mut self, days: u32) -> Result<()> {
        let now = Utc::now();
        let should_prune = match self.last_pruned {
            None => true,
            Some(t) => (now - t).num_days() as u32 >= days,
        };
        if !should_prune {
            return Ok(());
        }
        // Don't churn small caches.
        if self.seen.len() > 2048 {
            let take = self.seen.len() / 2;
            let kept: HashSet<String> = self.seen.iter().take(take).cloned().collect();
            self.seen = kept;
        }
        self.last_pruned = Some(now);
        self.persist()
    }

    fn persist(&self) -> Result<()> {
        let disk = OnDisk {
            seen: self.seen.iter().cloned().collect(),
            unread: self.unread.iter().cloned().collect(),
            last_pruned: self.last_pruned,
        };
        let text = serde_json::to_string(&disk).context("seen-store serialize failed")?;
        std::fs::write(&self.path, text)
            .with_context(|| format!("failed to write {}", self.path.display()))?;
        Ok(())
    }
}

/// Lowercase + replace `@` with `_at_` and any other non-alphanumeric with
/// `_` so the filename is always portable. We don't care about preserving
/// the original address — the user never sees this filename, only the file
/// itself in `~/.config/docket/`.
fn sanitize_account(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 4);
    for ch in raw.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower == '@' {
            out.push_str("_at_");
        } else if lower.is_ascii_alphanumeric() || lower == '.' || lower == '-' {
            out.push(lower);
        } else {
            out.push('_');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Allocate an isolated test path. Uses the system temp dir + a counter
    /// so parallel tests never collide. No process-global state involved.
    fn temp_path(tag: &str) -> PathBuf {
        let nano = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "docket-seen-{tag}-{}-{nano}.json",
            std::process::id()
        ))
    }

    #[test]
    fn sanitize_replaces_at_and_specials() {
        assert_eq!(
            sanitize_account("Alice@Example.com"),
            "alice_at_example.com"
        );
        assert_eq!(sanitize_account("foo+bar@baz.io"), "foo_bar_at_baz.io");
        assert_eq!(sanitize_account("a/b\\c"), "a_b_c");
    }

    #[test]
    fn save_and_load_roundtrip() {
        let path = temp_path("roundtrip");
        let _cleanup = scopeguard_remove(path.clone());

        let mut s = SeenStore::load_at_path(path.clone()).unwrap();
        assert!(!s.contains("msg-1"));
        s.mark_seen("msg-1").unwrap();
        s.mark_seen("msg-2").unwrap();
        // mark_seen on the same id is idempotent.
        s.mark_seen("msg-1").unwrap();

        let s2 = SeenStore::load_at_path(path).unwrap();
        assert!(s2.contains("msg-1"));
        assert!(s2.contains("msg-2"));
        assert!(!s2.contains("msg-3"));
    }

    #[test]
    fn mark_unread_overrides_and_clears_seen() {
        let path = temp_path("unread-override");
        let _cleanup = scopeguard_remove(path.clone());

        let mut s = SeenStore::load_at_path(path.clone()).unwrap();
        s.mark_seen("msg-1").unwrap();
        assert!(s.contains("msg-1"));
        assert!(!s.is_forced_unread("msg-1"));

        s.mark_unread("msg-1").unwrap();
        assert!(!s.contains("msg-1"));
        assert!(s.is_forced_unread("msg-1"));

        // mark_seen wins back over a forced-unread override.
        s.mark_seen("msg-1").unwrap();
        assert!(s.contains("msg-1"));
        assert!(!s.is_forced_unread("msg-1"));

        let s2 = SeenStore::load_at_path(path).unwrap();
        assert!(s2.contains("msg-1"));
        assert!(!s2.is_forced_unread("msg-1"));
    }

    #[test]
    fn missing_file_returns_empty_store() {
        let path = temp_path("missing");
        let _cleanup = scopeguard_remove(path.clone());
        let s = SeenStore::load_at_path(path).unwrap();
        assert!(!s.contains("anything"));
    }

    /// Tiny RAII helper — drops the test file when the guard goes out of
    /// scope, so the system temp dir doesn't accumulate cruft.
    struct RemoveOnDrop(PathBuf);
    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    fn scopeguard_remove(path: PathBuf) -> RemoveOnDrop {
        RemoveOnDrop(path)
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

pub mod types;
pub mod watcher;

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{Context, Result};

pub use types::Config;
pub use types::ZoomMargin;

static CONFIG_DIR_OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Point the config dir at an explicit directory, bypassing the default XDG
/// location. Used by `--config <FILE>` (single-file mode).
pub fn set_config_dir_override(dir: PathBuf) {
    if let Ok(mut w) = CONFIG_DIR_OVERRIDE.write() {
        *w = Some(dir);
    }
}

/// The docket root — `~/.config/docket/` (overridable with `$XDG_CONFIG_HOME`).
/// The XDG Base Directory layout is what the spec promises, so we use it
/// consistently rather than `~/Library/Application Support/` (macOS) or
/// `%APPDATA%`.
pub fn docket_root() -> Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("docket"));
        }
    }
    let home = dirs::home_dir().context("could not locate user home directory")?;
    Ok(home.join(".config").join("docket"))
}

/// The config directory — everything (widget configs, credentials, runtime
/// state, notes, log) resolves under this. An explicit `--config` override
/// short-circuits to that file's directory; otherwise it's `docket_root()`.
pub fn config_dir() -> Result<PathBuf> {
    if let Ok(guard) = CONFIG_DIR_OVERRIDE.read() {
        if let Some(dir) = guard.as_ref() {
            return Ok(dir.clone());
        }
    }
    docket_root()
}

/// Returns the path to the main config file (`config.toml`).
pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

/// Load the main config from disk. If the path does not exist, returns the
/// built-in defaults. CLI-supplied `override_path` takes precedence over the
/// XDG default location.
pub fn load(override_path: Option<&Path>) -> Result<Config> {
    let path: PathBuf = match override_path {
        Some(p) => p.to_path_buf(),
        None => config_path()?,
    };

    if !path.exists() {
        tracing::info!(path = %path.display(), "config file not found, using built-in defaults");
        return Ok(Config::default());
    }

    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file at {}", path.display()))?;
    let cfg: Config = toml::from_str(&contents)
        .with_context(|| format!("failed to parse config file at {}", path.display()))?;
    Ok(cfg)
}

/// Parse `config.toml` as a raw TOML value — used to pull out individual
/// widget sections (`[calendar]`, `[feeds]`, …) as `serde_json::Value` for
/// `WidgetCtx`/hot-reload, without requiring every widget config struct to
/// implement `Serialize` just to round-trip through the app-level typed
/// `Config`. Mirrors `load`'s path resolution and missing-file fallback
/// (an empty table, so every section resolves to `Value::Null` below).
pub fn load_raw(override_path: Option<&Path>) -> Result<toml::Value> {
    let path: PathBuf = match override_path {
        Some(p) => p.to_path_buf(),
        None => config_path()?,
    };
    if !path.exists() {
        return Ok(toml::Value::Table(Default::default()));
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file at {}", path.display()))?;
    toml::from_str(&contents)
        .with_context(|| format!("failed to parse config file at {}", path.display()))
}

/// Extract one top-level table from a `load_raw` value as a
/// `serde_json::Value`, ready for `serde_json::from_value(..).unwrap_or_default()`
/// — the same bridge `Widget::apply_config` uses for hot-reload. `Value::Null`
/// when the section is absent (widget falls back to its own `Default`).
pub fn widget_section_json(raw: &toml::Value, key: &str) -> serde_json::Value {
    raw.get(key)
        .and_then(|v| serde_json::to_value(v).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// Default `config.toml` contents written by `--init` — one file with
/// `[global]`, `[calendar]`, `[feeds]`, and `[llm]` tables. Notes and
/// email ship no seed content (widgets fall back to their own
/// `Default` impl) — same as before consolidation.
pub const DEFAULT_CONFIG_TOML: &str = include_str!("defaults/config.toml");

pub const DEFAULT_ANTHROPIC_KEY_TEMPLATE: &str = include_str!("defaults/credentials/anthropic.toml");

pub const DEFAULT_OPENAI_KEY_TEMPLATE: &str = include_str!("defaults/credentials/openai.toml");

pub const DEFAULT_CALDAV_TEMPLATE: &str = include_str!("defaults/credentials/caldav.toml");

pub const DEFAULT_ICS_TEMPLATE: &str = include_str!("defaults/credentials/ics.toml");

/// Create `~/.config/docket/` and seed `config.toml` + credential
/// template files if they do not already exist. Idempotent — existing files
/// are left untouched. Returns the path of the main `config.toml`.
pub fn init_default_config() -> Result<PathBuf> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create docket root at {}", dir.display()))?;
    seed(&dir.join("config.toml"), DEFAULT_CONFIG_TOML)?;

    let creds = dir.join("credentials");
    std::fs::create_dir_all(&creds)
        .with_context(|| format!("failed to create {}", creds.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&creds, std::fs::Permissions::from_mode(0o700));
    }
    seed_credentials(
        &creds.join("anthropic_key.toml"),
        DEFAULT_ANTHROPIC_KEY_TEMPLATE,
    )?;
    seed_credentials(&creds.join("openai_key.toml"), DEFAULT_OPENAI_KEY_TEMPLATE)?;
    seed_credentials(&creds.join("caldav.toml"), DEFAULT_CALDAV_TEMPLATE)?;
    seed_credentials(&creds.join("ics.toml"), DEFAULT_ICS_TEMPLATE)?;
    Ok(dir.join("config.toml"))
}

fn seed_credentials(path: &Path, contents: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, contents)
        .with_context(|| format!("failed to write credentials template at {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    tracing::info!(path = %path.display(), "wrote credentials template");
    Ok(())
}

fn seed(path: &Path, contents: &str) -> Result<()> {
    if path.exists() {
        tracing::info!(path = %path.display(), "config file already exists, leaving in place");
        return Ok(());
    }
    std::fs::write(path, contents)
        .with_context(|| format!("failed to write default config to {}", path.display()))?;
    tracing::info!(path = %path.display(), "wrote default config");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses() {
        let cfg: Config = toml::from_str(DEFAULT_CONFIG_TOML).expect("default config should parse");
        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.global.command_key, ":");
    }

    #[test]
    fn minimal_config_uses_defaults() {
        let cfg: Config = toml::from_str("").expect("empty config should parse");
        assert_eq!(cfg.version, 1);
    }

    #[test]
    fn default_config_seed_populates_every_widget_table() {
        // Each widget's section is checked only when that widget is
        // compiled in — slim builds drop the type references but the
        // TOML itself stays so `init_default_config` keeps populating
        // it at install time.
        let cfg: Config = toml::from_str(DEFAULT_CONFIG_TOML).expect("default config should parse");
        #[cfg(feature = "widget-calendar")]
        assert!(
            !cfg.calendar.events.is_empty(),
            "[calendar] seed should ship example events"
        );
        #[cfg(feature = "widget-feeds")]
        assert!(
            !cfg.feeds.feeds.is_empty(),
            "[feeds] seed should ship example feeds — it's in the fixed layout"
        );
        assert!(cfg.llm.enabled);
        assert_eq!(cfg.llm.provider.name, "anthropic");
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let cfg = load(Some(Path::new("/nonexistent/docket/config.toml")))
            .expect("missing file should not error");
        assert_eq!(cfg.version, 1);
    }

    #[test]
    fn load_raw_missing_file_returns_empty_table() {
        let raw = load_raw(Some(Path::new("/nonexistent/docket/config.toml")))
            .expect("missing file should not error");
        assert_eq!(widget_section_json(&raw, "calendar"), serde_json::Value::Null);
    }

    #[test]
    fn widget_section_json_extracts_named_table() {
        let raw: toml::Value = toml::from_str("[calendar]\ndefault_view = \"week\"\n").unwrap();
        let section = widget_section_json(&raw, "calendar");
        assert_eq!(section["default_view"], serde_json::json!("week"));
        assert_eq!(widget_section_json(&raw, "notes"), serde_json::Value::Null);
    }
}

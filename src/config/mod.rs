// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

pub mod layout;
pub mod types;
pub mod watcher;

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{Context, Result};

pub use layout::LayoutConfig;
pub use types::Config;
pub use types::ZoomMargin;

/// Load a per-widget TOML config from `~/.config/docket/<name>.toml`. Returns
/// `T::default()` if the file does not exist.
pub fn load_widget_toml<T>(name: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned + Default,
{
    let path = config_dir()?.join(format!("{name}.toml"));
    if !path.exists() {
        return Ok(T::default());
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read widget config at {}", path.display()))?;
    let value: T = toml::from_str(&contents)
        .with_context(|| format!("failed to parse widget config at {}", path.display()))?;
    Ok(value)
}

/// Like `load_widget_toml`, but resolves to `<kind>@<instance>.toml` for
/// non-main instances. Falls back to `T::default()` when the file doesn't
/// exist.
pub fn load_widget_toml_for_instance<T>(kind: &str, instance: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned + Default,
{
    let stem = crate::widgets::widget_config_stem(kind, instance);
    load_widget_toml(&stem)
}

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

/// Default `config.toml` contents written by `--init`.
pub const DEFAULT_CONFIG_TOML: &str = include_str!("defaults/config.toml");

pub const DEFAULT_NEWS_TOML: &str = include_str!("defaults/news.toml");

pub const DEFAULT_FEEDS_AI_TOML: &str = include_str!("defaults/feeds@ai.toml");

pub const DEFAULT_COLORSCHEMES_TOML: &str = include_str!("defaults/colorschemes.toml");

pub const DEFAULT_LLM_TOML: &str = include_str!("defaults/llm.toml");

pub const DEFAULT_ANTHROPIC_KEY_TEMPLATE: &str = include_str!("defaults/credentials/anthropic.toml");

pub const DEFAULT_OPENAI_KEY_TEMPLATE: &str = include_str!("defaults/credentials/openai.toml");

pub const DEFAULT_CALDAV_TEMPLATE: &str = include_str!("defaults/credentials/caldav.toml");

pub const DEFAULT_ICS_TEMPLATE: &str = include_str!("defaults/credentials/ics.toml");

pub const DEFAULT_CALENDAR_TOML: &str = include_str!("defaults/calendar.toml");

/// Create `~/.config/docket/` and seed the default config + credential
/// template files if they do not already exist. Idempotent — existing files
/// are left untouched. Returns the path of the main `config.toml`.
pub fn init_default_config() -> Result<PathBuf> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create docket root at {}", dir.display()))?;
    seed(&dir.join("colorschemes.toml"), DEFAULT_COLORSCHEMES_TOML)?;
    seed(&dir.join("config.toml"), DEFAULT_CONFIG_TOML)?;
    seed(&dir.join("calendar.toml"), DEFAULT_CALENDAR_TOML)?;
    seed(&dir.join("news.toml"), DEFAULT_NEWS_TOML)?;
    seed(&dir.join("feeds@ai.toml"), DEFAULT_FEEDS_AI_TOML)?;
    seed(&dir.join("llm.toml"), DEFAULT_LLM_TOML)?;

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
        assert_eq!(cfg.layout.cells.len(), 4);
        assert_eq!(cfg.global.command_key, ":");
    }

    #[test]
    fn minimal_config_uses_defaults() {
        let cfg: Config = toml::from_str("").expect("empty config should parse");
        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.layout.cells.len(), 4);
    }

    #[test]
    fn default_colorschemes_seed_parses_and_has_default_scheme() {
        let file: crate::theme::ColorSchemesFile =
            toml::from_str(DEFAULT_COLORSCHEMES_TOML).expect("colorschemes seed should parse");
        assert!(
            file.schemes.contains_key("default"),
            "default scheme must exist so the unmodified config.toml resolves"
        );
        for expected in [
            "chalktone",
            "gruvbox",
            "tokyonight",
            "rosepine",
            "nord",
            "bluloco",
            "onedark",
            "miasma",
        ] {
            assert!(
                file.schemes.contains_key(expected),
                "expected scheme {expected:?} in seed"
            );
        }
    }

    #[test]
    fn seeded_schemes_populate_every_themable_role() {
        // Guards against the quoted-dotted-key bug (`"border.focused"`
        // silently parses as a single key) AND against new roles being
        // added without each seeded scheme being updated. Every scheme
        // ships values for every role exposed in colorschemes.toml.
        let file: crate::theme::ColorSchemesFile =
            toml::from_str(DEFAULT_COLORSCHEMES_TOML).expect("seed parses");
        for (name, scheme) in &file.schemes {
            assert!(
                scheme.border.focused.is_some(),
                "scheme {name:?} should set border.focused (use unquoted dotted keys)"
            );
            assert!(
                scheme.widget_title.focused.is_some(),
                "scheme {name:?} should set widget_title.focused"
            );
            assert!(
                scheme.widget_title.unfocused.is_some(),
                "scheme {name:?} should set widget_title.unfocused"
            );
            assert!(
                scheme.metadata.focused.is_some(),
                "scheme {name:?} should set metadata.focused"
            );
            assert!(
                scheme.metadata.unfocused.is_some(),
                "scheme {name:?} should set metadata.unfocused"
            );
            assert!(
                scheme.text.focused.is_some(),
                "scheme {name:?} should set text.focused (use unquoted dotted keys)"
            );
        }
    }

    #[test]
    fn default_widget_seed_files_parse() {
        // Each widget's seed is checked only when that widget is compiled
        // in — slim builds drop the type references but the TOML strings
        // themselves stay so `seed_defaults` keeps populating them at
        // install time.
        #[cfg(feature = "widget-clock")]
        {
            let _: crate::widgets::clock::ClockConfig =
                toml::from_str(DEFAULT_CLOCK_TOML).expect("clock seed should parse");
        }
        #[cfg(feature = "widget-weather")]
        {
            let _: crate::widgets::weather::WeatherConfig =
                toml::from_str(DEFAULT_WEATHER_TOML).expect("weather seed should parse");
        }
        #[cfg(feature = "widget-calendar")]
        {
            let cal: crate::widgets::calendar::CalendarConfig =
                toml::from_str(DEFAULT_CALENDAR_TOML).expect("calendar seed should parse");
            assert!(
                !cal.events.is_empty(),
                "calendar seed should ship example events"
            );
        }
        #[cfg(feature = "widget-news")]
        {
            let news: crate::widgets::news::NewsConfig =
                toml::from_str(DEFAULT_NEWS_TOML).expect("news seed should parse");
            assert!(
                !news.feeds.is_empty(),
                "news seed should ship example feeds"
            );
        }
        #[cfg(feature = "widget-feeds")]
        {
            let ai: crate::widgets::feeds::FeedsConfig =
                toml::from_str(DEFAULT_FEEDS_AI_TOML).expect("feeds@ai seed should parse");
            assert!(
                !ai.feeds.is_empty(),
                "feeds@ai seed should ship example feeds — it's in the default layout"
            );
        }
        let llm: crate::llm::LlmConfig =
            toml::from_str(DEFAULT_LLM_TOML).expect("llm seed should parse");
        assert!(llm.enabled);
        assert_eq!(llm.provider.name, "anthropic");
        #[cfg(feature = "widget-stocks")]
        {
            let stocks: crate::widgets::stocks::StocksConfig =
                toml::from_str(DEFAULT_STOCKS_TOML).expect("stocks seed should parse");
            assert!(!stocks.indices.is_empty());
            assert!(!stocks.watchlist.is_empty());
        }
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let cfg = load(Some(Path::new("/nonexistent/docket/config.toml")))
            .expect("missing file should not error");
        assert_eq!(cfg.version, 1);
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

use anyhow::{bail, Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use std::path::PathBuf;
use toml_edit::{DocumentMut, Item, Table};

const TABS: [&str; 4] = ["Email", "Calendar", "Feeds", "Notes"];

#[derive(Clone)]
struct Field {
    section: &'static str,
    array: Option<(&'static str, usize)>,
    key: &'static str,
    label: String,
    description: &'static str,
    value: String,
    original: String,
    integer: bool,
    list: bool,
    credential: Option<usize>,
    secret: bool,
}

struct Credential {
    kind: &'static str,
    path: PathBuf,
    original: Option<String>,
    doc: DocumentMut,
}

pub struct Settings {
    path: PathBuf,
    original: String,
    doc: DocumentMut,
    fields: Vec<Vec<Field>>,
    credentials: Vec<Credential>,
    tab: usize,
    row: usize,
    editing: bool,
    saved: bool,
    error: Option<String>,
}

impl Settings {
    pub fn open(path: PathBuf) -> Result<Self> {
        let original = std::fs::read_to_string(&path)
            .with_context(|| format!("Could not read {}", path.display()))?;
        let doc: DocumentMut = original.parse().context("Invalid configuration")?;
        let mut fields = vec![Vec::new(); 4];
        let mut add = |tab: usize,
                       section: &'static str,
                       array: Option<(&'static str, usize)>,
                       key: &'static str,
                       label: String,
                       description: &'static str,
                       fallback: &str,
                       integer: bool,
                       list: bool| {
            let item = if let Some((name, index)) = array {
                doc.get(section)
                    .and_then(|v| v.get(name))
                    .and_then(Item::as_array_of_tables)
                    .and_then(|a| a.get(index))
                    .and_then(|t| t.get(key))
            } else {
                doc.get(section).and_then(|v| v.get(key))
            };
            let value = item
                .map(|v| {
                    if list {
                        v.as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default()
                    } else if integer {
                        v.as_integer()
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| fallback.into())
                    } else {
                        v.as_str().unwrap_or(fallback).to_owned()
                    }
                })
                .unwrap_or_else(|| fallback.into());
            fields[tab].push(Field {
                section,
                array,
                key,
                label,
                description,
                original: value.clone(),
                value,
                integer,
                list,
                credential: None,
                secret: false,
            });
        };
        add(
            0,
            "email",
            None,
            "provider",
            "Provider".into(),
            "imap enables email; none disables it.",
            "imap",
            false,
            false,
        );
        add(
            0,
            "email",
            None,
            "latest_days",
            "Days to show".into(),
            "Fetch messages from the past N days.",
            "7",
            true,
            false,
        );
        add(
            0,
            "email",
            None,
            "refresh_minutes",
            "Refresh minutes".into(),
            "Minutes between inbox refreshes.",
            "5",
            true,
            false,
        );
        add(
            0,
            "email",
            None,
            "folders",
            "Folders".into(),
            "Comma-separated IMAP folders for a single account.",
            "INBOX",
            false,
            true,
        );
        if let Some(accounts) = doc
            .get("email")
            .and_then(|t| t.get("accounts"))
            .and_then(Item::as_array_of_tables)
        {
            for (i, account) in accounts.iter().enumerate() {
                let label = account
                    .get("label")
                    .and_then(Item::as_str)
                    .unwrap_or("account");
                add(
                    0,
                    "email",
                    Some(("accounts", i)),
                    "folders",
                    format!("{label} folders"),
                    "Comma-separated folders for this account.",
                    "INBOX",
                    false,
                    true,
                );
            }
        }
        add(
            1,
            "calendar",
            None,
            "default_view",
            "Default view".into(),
            "day, week or month.",
            "day",
            false,
            false,
        );
        add(
            1,
            "calendar",
            None,
            "poll_interval_secs",
            "Refresh seconds".into(),
            "Seconds between calendar refreshes.",
            "60",
            true,
            false,
        );
        add(
            1,
            "calendar",
            None,
            "first_day_of_week",
            "Week starts".into(),
            "Weekday name, such as monday or sunday.",
            "sunday",
            false,
            false,
        );
        add(
            1,
            "calendar",
            None,
            "local_ics_path",
            "Local calendar".into(),
            "Path to a local .ics file; empty uses the default.",
            "",
            false,
            false,
        );
        if let Some(providers) = doc
            .get("calendar")
            .and_then(|t| t.get("providers"))
            .and_then(Item::as_array_of_tables)
        {
            for (i, _) in providers.iter().enumerate() {
                add(
                    1,
                    "calendar",
                    Some(("providers", i)),
                    "kind",
                    format!("Source {} type", i + 1),
                    "local, caldav or ics.",
                    "local",
                    false,
                    false,
                );
                add(
                    1,
                    "calendar",
                    Some(("providers", i)),
                    "account",
                    format!("Source {} account", i + 1),
                    "ICS feed label or CalDAV account.",
                    "",
                    false,
                    false,
                );
            }
        }
        add(
            2,
            "feeds",
            None,
            "poll_interval_secs",
            "Refresh seconds".into(),
            "Seconds between feed refreshes.",
            "900",
            true,
            false,
        );
        if let Some(feeds) = doc
            .get("feeds")
            .and_then(|t| t.get("feeds"))
            .and_then(Item::as_array_of_tables)
        {
            for (i, _) in feeds.iter().enumerate() {
                add(
                    2,
                    "feeds",
                    Some(("feeds", i)),
                    "topic",
                    format!("Feed {} name", i + 1),
                    "Name shown on the feed tab.",
                    "",
                    false,
                    false,
                );
                add(
                    2,
                    "feeds",
                    Some(("feeds", i)),
                    "url",
                    format!("Feed {} URL", i + 1),
                    "RSS or Atom feed address.",
                    "",
                    false,
                    false,
                );
            }
        }
        add(
            3,
            "notes",
            None,
            "notes_dir",
            "Notes directory".into(),
            "Markdown notes folder; empty uses the default.",
            "",
            false,
            false,
        );
        drop(add);
        let mut credentials = Vec::new();
        let labels: Vec<String> = doc
            .get("email")
            .and_then(|t| t.get("accounts"))
            .and_then(Item::as_array_of_tables)
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.get("label").and_then(Item::as_str).map(str::to_owned))
                    .collect()
            })
            .filter(|v: &Vec<String>| !v.is_empty())
            .unwrap_or_else(|| vec![String::new()]);
        let credential_dir = path
            .parent()
            .context("Settings file has no directory")?
            .join("credentials");
        for label in labels {
            if !label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                continue;
            }
            let filename = if label.is_empty() {
                "imap.toml".to_string()
            } else {
                format!("imap_{label}.toml")
            };
            let credential_path = credential_dir.join(filename);
            let original_credential = match std::fs::read_to_string(&credential_path) {
                Ok(text) => Some(text),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            let credential_doc: DocumentMut = original_credential
                .as_deref()
                .unwrap_or("")
                .parse()
                .context("Invalid IMAP credentials")?;
            let index = credentials.len();
            for (key, name, description, secret, integer) in [
                (
                    "host",
                    "IMAP host",
                    "Mail server hostname, such as imap.gmail.com.",
                    false,
                    false,
                ),
                (
                    "port",
                    "IMAP port",
                    "IMAP server port; 993 is the usual TLS port.",
                    false,
                    true,
                ),
                (
                    "username",
                    "Username",
                    "IMAP username or email address.",
                    false,
                    false,
                ),
                (
                    "app_password",
                    "App password",
                    "Password is hidden; use an app-specific password.",
                    true,
                    false,
                ),
            ] {
                let value = if integer {
                    credential_doc
                        .get(key)
                        .and_then(Item::as_integer)
                        .unwrap_or(993)
                        .to_string()
                } else {
                    credential_doc
                        .get(key)
                        .and_then(Item::as_str)
                        .unwrap_or("")
                        .to_string()
                };
                fields[0].push(Field {
                    section: "email",
                    array: None,
                    key,
                    label: if label.is_empty() {
                        name.into()
                    } else {
                        format!("{label} {name}")
                    },
                    description,
                    original: value.clone(),
                    value,
                    integer,
                    list: false,
                    credential: Some(index),
                    secret,
                });
            }
            credentials.push(Credential {
                kind: "imap",
                path: credential_path,
                original: original_credential,
                doc: credential_doc,
            });
        }
        let caldav_path = credential_dir.join("caldav.toml");
        let caldav_original = match std::fs::read_to_string(&caldav_path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let caldav_doc: DocumentMut = caldav_original
            .as_deref()
            .unwrap_or("")
            .parse()
            .context("Invalid CalDAV credentials")?;
        let caldav_index = credentials.len();
        for (key, name, description, secret) in [
            ("server", "CalDAV server", "CalDAV server URL.", false),
            (
                "username",
                "CalDAV username",
                "Username for the CalDAV server.",
                false,
            ),
            (
                "app_password",
                "CalDAV password",
                "Password is hidden; use an app-specific password.",
                true,
            ),
        ] {
            let value = caldav_doc
                .get(key)
                .and_then(Item::as_str)
                .unwrap_or("")
                .to_string();
            fields[1].push(Field {
                section: "calendar",
                array: None,
                key,
                label: name.into(),
                description,
                original: value.clone(),
                value,
                integer: false,
                list: false,
                credential: Some(caldav_index),
                secret,
            });
        }
        credentials.push(Credential {
            kind: "caldav",
            path: caldav_path,
            original: caldav_original,
            doc: caldav_doc,
        });
        let ics_path = credential_dir.join("ics.toml");
        let ics_original = match std::fs::read_to_string(&ics_path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let ics_doc: DocumentMut = ics_original
            .as_deref()
            .unwrap_or("")
            .parse()
            .context("Invalid ICS credentials")?;
        let ics_index = credentials.len();
        if let Some(feeds) = ics_doc.get("feeds").and_then(Item::as_array_of_tables) {
            for (i, feed) in feeds.iter().enumerate() {
                for (key, name, description, secret) in [
                    (
                        "label",
                        "ICS label",
                        "Label referenced by a calendar source.",
                        false,
                    ),
                    (
                        "url",
                        "ICS URL",
                        "Private iCalendar feed URL; hidden for privacy.",
                        true,
                    ),
                ] {
                    let value = feed
                        .get(key)
                        .and_then(Item::as_str)
                        .unwrap_or("")
                        .to_string();
                    fields[1].push(Field {
                        section: "calendar",
                        array: Some(("feeds", i)),
                        key,
                        label: format!("{name} {}", i + 1),
                        description,
                        original: value.clone(),
                        value,
                        integer: false,
                        list: false,
                        credential: Some(ics_index),
                        secret,
                    });
                }
            }
        }
        credentials.push(Credential {
            kind: "ics",
            path: ics_path,
            original: ics_original,
            doc: ics_doc,
        });
        Ok(Self {
            path,
            original,
            doc,
            fields,
            credentials,
            tab: 0,
            row: 0,
            editing: false,
            saved: false,
            error: None,
        })
    }

    pub fn saved(&self) -> bool {
        self.saved
    }

    pub fn insert(&mut self, text: &str) {
        if self.editing {
            self.fields[self.tab][self.row]
                .value
                .extend(text.chars().filter(|c| !c.is_control()));
        }
    }

    fn add_calendar_source(&mut self) {
        if !self.doc.contains_key("calendar") {
            self.doc["calendar"] = Item::Table(Table::new());
        }
        if self.doc["calendar"].get("providers").is_none() {
            self.doc["calendar"]["providers"] =
                Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
        }
        let Some(providers) = self.doc["calendar"]["providers"].as_array_of_tables_mut() else {
            self.error = Some("Calendar sources are not a list".into());
            return;
        };
        let index = providers.len();
        let mut table = Table::new();
        table["kind"] = toml_edit::value("caldav");
        providers.push(table);
        self.fields[1].push(Field {
            section: "calendar",
            array: Some(("providers", index)),
            key: "kind",
            label: format!("Source {} type", index + 1),
            description: "local, caldav or ics.",
            value: "caldav".into(),
            original: "caldav".into(),
            integer: false,
            list: false,
            credential: None,
            secret: false,
        });
        self.fields[1].push(Field {
            section: "calendar",
            array: Some(("providers", index)),
            key: "account",
            label: format!("Source {} account", index + 1),
            description: "ICS feed label; leave blank for default.",
            value: String::new(),
            original: String::new(),
            integer: false,
            list: false,
            credential: None,
            secret: false,
        });
        self.row = self.fields[1].len() - 2;
    }

    fn add_feed(&mut self) {
        if !self.doc.contains_key("feeds") {
            self.doc["feeds"] = Item::Table(Table::new());
        }
        if self.doc["feeds"].get("feeds").is_none() {
            self.doc["feeds"]["feeds"] = Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
        }
        let Some(feeds) = self.doc["feeds"]["feeds"].as_array_of_tables_mut() else {
            self.error = Some("Feed configuration is not a list".into());
            return;
        };
        let index = feeds.len();
        let mut table = Table::new();
        table["topic"] = toml_edit::value("");
        table["url"] = toml_edit::value("");
        feeds.push(table);
        for (key, label, description) in [
            (
                "topic",
                format!("Feed {} name", index + 1),
                "Name shown on the feed tab.",
            ),
            (
                "url",
                format!("Feed {} URL", index + 1),
                "RSS or Atom feed address.",
            ),
        ] {
            self.fields[2].push(Field {
                section: "feeds",
                array: Some(("feeds", index)),
                key,
                label,
                description,
                value: String::new(),
                original: String::new(),
                integer: false,
                list: false,
                credential: None,
                secret: false,
            });
        }
        self.row = self.fields[2].len() - 2;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        self.error = None;
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => match self.save() {
                Ok(()) => {
                    self.saved = true;
                    return true;
                }
                Err(e) => self.error = Some(format!("{e:#}")),
            },
            (_, KeyCode::Esc) if self.editing => self.editing = false,
            (_, KeyCode::Esc) => return true,
            (_, KeyCode::Enter) => self.editing = !self.editing,
            (_, KeyCode::Tab) if !self.editing => {
                self.tab = (self.tab + 1) % TABS.len();
                self.row = 0;
            }
            (_, KeyCode::BackTab) if !self.editing => {
                self.tab = (self.tab + TABS.len() - 1) % TABS.len();
                self.row = 0;
            }
            (_, KeyCode::Char('n')) if !self.editing && self.tab == 2 => self.add_feed(),
            (_, KeyCode::Char('n')) if !self.editing && self.tab == 1 => self.add_calendar_source(),
            (_, KeyCode::Up) if !self.editing => self.row = self.row.saturating_sub(1),
            (_, KeyCode::Down) if !self.editing => {
                self.row = (self.row + 1).min(self.fields[self.tab].len() - 1)
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u')) if self.editing => {
                self.fields[self.tab][self.row].value.clear();
            }
            (_, KeyCode::Backspace) if self.editing => {
                self.fields[self.tab][self.row].value.pop();
            }
            (m, KeyCode::Char(c)) if self.editing && (m.is_empty() || m == KeyModifiers::SHIFT) => {
                self.fields[self.tab][self.row].value.push(c)
            }
            _ => {}
        }
        false
    }

    fn save(&mut self) -> Result<()> {
        if std::fs::read_to_string(&self.path)? != self.original {
            bail!("Settings changed on disk. Reopen before saving.");
        }
        let mut doc = self.doc.clone();
        let mut credential_docs: Vec<DocumentMut> =
            self.credentials.iter().map(|c| c.doc.clone()).collect();
        for field in self
            .fields
            .iter()
            .flatten()
            .filter(|f| f.credential.is_some() && f.value != f.original)
        {
            let credential = &mut credential_docs[field.credential.unwrap()];
            if let Some((array, index)) = field.array {
                credential[array]
                    .as_array_of_tables_mut()
                    .and_then(|a| a.get_mut(index))
                    .context("Calendar feed no longer exists")?[field.key] =
                    toml_edit::value(field.value.as_str());
            } else {
                credential[field.key] = if field.integer {
                    let port: u16 = field
                        .value
                        .trim()
                        .parse()
                        .context("IMAP port must be between 1 and 65535")?;
                    if port == 0 {
                        bail!("IMAP port must be between 1 and 65535");
                    }
                    toml_edit::value(i64::from(port))
                } else {
                    toml_edit::value(field.value.as_str())
                };
            }
        }
        for (i, credential) in self.credentials.iter().enumerate() {
            if credential_docs[i].to_string() == credential.doc.to_string() {
                continue;
            }
            let current = match std::fs::read_to_string(&credential.path) {
                Ok(text) => Some(text),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            if current != credential.original {
                bail!("Email credentials changed on disk. Reopen before saving.");
            }
        }
        for field in self
            .fields
            .iter()
            .flatten()
            .filter(|f| f.credential.is_none() && f.value != f.original)
        {
            let value = if field.integer {
                let number: i64 = field
                    .value
                    .trim()
                    .parse()
                    .with_context(|| format!("{} must be a number", field.label))?;
                if number <= 0 {
                    bail!("{} must be positive", field.label);
                }
                toml_edit::value(number)
            } else if field.list {
                let mut list = toml_edit::Array::new();
                for item in field
                    .value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    list.push(item);
                }
                toml_edit::value(list)
            } else {
                toml_edit::value(field.value.as_str())
            };
            if !doc.contains_key(field.section) {
                doc[field.section] = Item::Table(Table::new());
            }
            if let Some((name, index)) = field.array {
                let table = doc[field.section][name]
                    .as_array_of_tables_mut()
                    .and_then(|a| a.get_mut(index))
                    .context("Source no longer exists")?;
                table[field.key] = value;
            } else {
                doc[field.section][field.key] = value;
            }
        }
        if let Some(feeds) = doc
            .get("feeds")
            .and_then(|t| t.get("feeds"))
            .and_then(Item::as_array_of_tables)
        {
            for feed in feeds {
                if ["topic", "url"].iter().any(|key| {
                    feed.get(key)
                        .and_then(Item::as_str)
                        .unwrap_or("")
                        .trim()
                        .is_empty()
                }) {
                    bail!("Each feed needs a name and URL");
                }
            }
        }
        let text = doc.to_string();
        let _: crate::config::Config = toml::from_str(&text).context("Invalid settings")?;
        for (i, credential) in self.credentials.iter().enumerate() {
            if credential_docs[i].to_string() == credential.doc.to_string() {
                continue;
            }
            let cred_doc = &mut credential_docs[i];
            if credential.kind == "imap" && !cred_doc.contains_key("port") {
                cred_doc["port"] = toml_edit::value(993);
            }
            if credential.kind == "imap" && !cred_doc.contains_key("use_tls") {
                cred_doc["use_tls"] = toml_edit::value(true);
            }
            let required: &[&str] = match credential.kind {
                "imap" => &["host", "username", "app_password"],
                "caldav" => &["server", "username", "app_password"],
                _ => &[],
            };
            for &key in required {
                if cred_doc
                    .get(key)
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
                    bail!("Server, username and app password are required");
                }
            }
        }
        for (i, credential) in self.credentials.iter().enumerate() {
            if credential_docs[i].to_string() == credential.doc.to_string() {
                continue;
            }
            let cred_doc = &credential_docs[i];
            let dir = credential
                .path
                .parent()
                .context("Credential path has no directory")?;
            std::fs::create_dir_all(dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
            }
            let tmp = credential
                .path
                .with_extension(format!("toml.{}.tmp", std::process::id()));
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp)?;
            let result = (|| -> Result<()> {
                file.write_all(cred_doc.to_string().as_bytes())?;
                file.sync_all()?;
                std::fs::rename(&tmp, &credential.path)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            result?;
        }
        let tmp = self
            .path
            .with_extension(format!("toml.{}.tmp", std::process::id()));
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        let result = (|| -> Result<()> {
            file.set_permissions(std::fs::metadata(&self.path)?.permissions())?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&tmp, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result?;
        self.original = text;
        self.doc = doc;
        Ok(())
    }

    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        if area.width < 20 || area.height < 8 {
            return;
        }
        let rect = Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2);
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::LightCyan))
            .title(" Settings ");
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let tabs = Line::from(
            TABS.iter()
                .enumerate()
                .map(|(i, t)| {
                    Span::styled(
                        format!(" {t} "),
                        if i == self.tab {
                            Style::default()
                                .fg(Color::Black)
                                .bg(Color::LightCyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::LightMagenta)
                        },
                    )
                })
                .collect::<Vec<_>>(),
        );
        frame.render_widget(
            Paragraph::new(tabs),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        let h = inner.height.saturating_sub(4) as usize;
        let top = self.row.saturating_sub(h.saturating_sub(1));
        let lines = self.fields[self.tab]
            .iter()
            .enumerate()
            .skip(top)
            .take(h)
            .map(|(i, f)| {
                let style = if i == self.row {
                    Style::default().fg(Color::Black).bg(Color::LightYellow)
                } else {
                    Style::default()
                };
                Line::from(format!(
                    " {:<22} {}",
                    f.label,
                    if f.secret {
                        "*".repeat(f.value.chars().count())
                    } else {
                        f.value.clone()
                    }
                ))
                .style(style)
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(lines),
            Rect::new(inner.x, inner.y + 1, inner.width, h as u16),
        );
        let desc = self
            .error
            .as_deref()
            .unwrap_or(self.fields[self.tab][self.row].description);
        frame.render_widget(
            Paragraph::new(desc).style(Style::default().fg(if self.error.is_some() {
                Color::LightRed
            } else {
                Color::LightCyan
            })),
            Rect::new(inner.x, inner.y + inner.height - 3, inner.width, 1),
        );
        let hint = if self.editing {
            "Type · Ctrl+U clear · Enter done · Ctrl+S save"
        } else if self.tab == 1 || self.tab == 2 {
            "Tab switch · ↑/↓ select · Enter edit · n add source · Ctrl+S save · Esc cancel"
        } else {
            "Tab switch · ↑/↓ select · Enter edit · Ctrl+S save · Esc cancel"
        };
        frame.render_widget(
            Paragraph::new(hint).style(Style::default().fg(Color::LightYellow)),
            Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn fixture() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "docket-settings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let path = path.join("config.toml");
        std::fs::write(&path, "# preserve me\n[email]\nprovider = \"imap\"\nrefresh_minutes = 5\n[[email.accounts]]\nlabel = \"work\"\nfolders = [\"INBOX\"]\n[notes]\nnotes_dir = \"/notes\"\n").unwrap();
        path
    }

    #[test]
    fn form_save_preserves_other_settings_and_comments() {
        let path = fixture();
        let mut settings = Settings::open(path.clone()).unwrap();
        let f = settings.fields[0]
            .iter_mut()
            .find(|f| f.key == "refresh_minutes")
            .unwrap();
        f.value = "10".into();
        settings.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# preserve me"));
        let doc: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(doc["email"]["refresh_minutes"].as_integer(), Some(10));
        assert_eq!(doc["email"]["accounts"][0]["label"].as_str(), Some("work"));
        assert_eq!(doc["notes"]["notes_dir"].as_str(), Some("/notes"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn rejects_conflicts_and_invalid_values() {
        let path = fixture();
        let mut settings = Settings::open(path.clone()).unwrap();
        settings.fields[0]
            .iter_mut()
            .find(|f| f.key == "refresh_minutes")
            .unwrap()
            .value = "no".into();
        assert!(settings.save().is_err());
        settings.fields[0]
            .iter_mut()
            .find(|f| f.key == "refresh_minutes")
            .unwrap()
            .value = "8".into();
        std::fs::write(&path, "[notes]\nnotes_dir = \"elsewhere\"\n").unwrap();
        assert!(settings.save().is_err());
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("elsewhere"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn writes_email_credentials_with_private_permissions() {
        let path = fixture();
        let mut settings = Settings::open(path.clone()).unwrap();
        for (key, value) in [
            ("host", "imap.example.com"),
            ("username", "user@example.com"),
            ("app_password", "test-secret"),
        ] {
            let field = settings.fields[0]
                .iter_mut()
                .find(|f| f.credential.is_some() && f.key == key)
                .unwrap();
            field.value = value.into();
            if key == "app_password" {
                assert!(field.secret);
            }
        }
        settings.save().unwrap();
        let credential = path.parent().unwrap().join("credentials/imap_work.toml");
        let doc: toml::Value =
            toml::from_str(&std::fs::read_to_string(&credential).unwrap()).unwrap();
        assert_eq!(doc["username"].as_str(), Some("user@example.com"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(credential).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn adds_a_feed_without_replacing_existing_settings() {
        let path = fixture();
        let mut settings = Settings::open(path.clone()).unwrap();
        settings.add_feed();
        let n = settings.fields[2].len();
        settings.fields[2][n - 2].value = "News".into();
        settings.fields[2][n - 1].value = "https://example.com/feed".into();
        settings.save().unwrap();
        let doc: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc["feeds"]["feeds"][0]["topic"].as_str(), Some("News"));
        assert_eq!(doc["notes"]["notes_dir"].as_str(), Some("/notes"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn calendar_credentials_are_kept_out_of_main_config() {
        let path = fixture();
        let mut settings = Settings::open(path.clone()).unwrap();
        for (key, value) in [
            ("server", "https://calendar.example.com"),
            ("username", "person"),
            ("app_password", "private"),
        ] {
            settings.fields[1]
                .iter_mut()
                .find(|f| f.credential.is_some() && f.key == key)
                .unwrap()
                .value = value.into();
        }
        settings.save().unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("private"));
        let credentials = path.parent().unwrap().join("credentials/caldav.toml");
        assert!(std::fs::read_to_string(credentials)
            .unwrap()
            .contains("private"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn rejects_external_credential_changes() {
        let path = fixture();
        let directory = path.parent().unwrap().join("credentials");
        std::fs::create_dir(&directory).unwrap();
        let credential = directory.join("imap_work.toml");
        std::fs::write(
            &credential,
            "host = \"old\"\nusername = \"u\"\napp_password = \"p\"\n",
        )
        .unwrap();
        let mut settings = Settings::open(path.clone()).unwrap();
        settings.fields[0]
            .iter_mut()
            .find(|f| f.credential.is_some() && f.key == "host")
            .unwrap()
            .value = "new".into();
        std::fs::write(&credential, "host = \"external\"\n").unwrap();
        assert!(settings.save().is_err());
        assert!(std::fs::read_to_string(credential)
            .unwrap()
            .contains("external"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn edits_existing_ics_url_without_moving_it_into_config() {
        let path = fixture();
        let directory = path.parent().unwrap().join("credentials");
        std::fs::create_dir(&directory).unwrap();
        let ics = directory.join("ics.toml");
        std::fs::write(
            &ics,
            "[[feeds]]\nlabel = \"work\"\nurl = \"https://old.example\"\n",
        )
        .unwrap();
        let mut settings = Settings::open(path.clone()).unwrap();
        let field = settings.fields[1]
            .iter_mut()
            .find(|f| f.credential.is_some() && f.key == "url")
            .unwrap();
        assert!(field.secret);
        field.value = "https://new.example".into();
        settings.save().unwrap();
        assert!(std::fs::read_to_string(ics)
            .unwrap()
            .contains("https://new.example"));
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("https://new.example"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}

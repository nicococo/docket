// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

use anyhow::{bail, Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use std::{
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
    thread,
};
use toml_edit::{DocumentMut, Item, Table};

const TABS: [&str; 5] = ["Email", "Calendar", "Feeds", "Notes", "Herdr"];
const HERDR_ACTION: &str = "nicococo.docket.toggle";

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
    herdr_path: PathBuf,
    herdr_original: Option<String>,
    herdr_doc: DocumentMut,
    herdr_bindings: Vec<String>,
    herdr_new_binding: String,
    herdr_update_status: Arc<Mutex<String>>,
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
            "Use imap to enable email. Add a provider in-app, then enter its IMAP server below; Gmail needs an app password.",
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
            "Comma-separated folders such as INBOX, Sent. Find exact names in your mail app's IMAP settings.",
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
            "Choose day, week or month. This is the view shown when docket opens.",
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
            "Weekday name. Use monday for ISO weeks or sunday for US-style weeks.",
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
            "Path to a local .ics calendar file; leave blank to use docket's example calendar.",
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
                    "Choose local, caldav or ics. CalDAV credentials go in the fields below; ICS URL goes in credentials/ics.toml.",
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
                    "ICS label from credentials/ics.toml; leave blank for default CalDAV discovery.",
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
                    "Short tab name. Paste the RSS/Atom feed URL in the next field.",
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
                    "RSS/Atom URL; copy the feed address from the publisher's website.",
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
            "Folder for Markdown notes. Leave blank to use ~/.config/docket/notes/.",
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
        let herdr_path = std::env::var_os("HERDR_CONFIG_PATH")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join("herdr/config.toml"))
            })
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join(".config/herdr/config.toml"))
            })
            .context("Could not locate Herdr config")?;
        let herdr_original = match std::fs::read_to_string(&herdr_path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let herdr_doc: DocumentMut = herdr_original
            .as_deref()
            .unwrap_or("")
            .parse()
            .context("Invalid Herdr configuration")?;
        let herdr_bindings = herdr_bindings(&herdr_doc);
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
            herdr_path,
            herdr_original,
            herdr_doc,
            herdr_bindings,
            herdr_new_binding: String::new(),
            herdr_update_status: Arc::new(Mutex::new(String::new())),
        })
    }

    pub fn saved(&self) -> bool {
        self.saved
    }

    pub fn insert(&mut self, text: &str) {
        if self.editing {
            if self.tab == 4 {
                self.herdr_new_binding.extend(
                    text.chars()
                        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '_')),
                );
            } else {
                self.fields[self.tab][self.row]
                    .value
                    .extend(text.chars().filter(|c| !c.is_control()));
            }
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

    fn add_email_account(&mut self) {
        if !self.doc.contains_key("email") {
            self.doc["email"] = Item::Table(Table::new());
        }
        if self.doc["email"].get("accounts").is_none() {
            self.doc["email"]["accounts"] = Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
        }
        let is_empty = self.doc["email"]["accounts"]
            .as_array_of_tables()
            .is_some_and(|a| a.is_empty());
        if is_empty {
            let folders = self
                .doc
                .get("email")
                .and_then(|e| e.get("folders"))
                .and_then(Item::as_array)
                .cloned()
                .unwrap_or_else(|| toml_edit::Array::from_iter(["INBOX"]));
            let mut legacy = Table::new();
            legacy["label"] = toml_edit::value("");
            legacy["folders"] = toml_edit::value(folders.clone());
            self.doc["email"]["accounts"]
                .as_array_of_tables_mut()
                .unwrap()
                .push(legacy);
            self.fields[0].retain(|field| !(field.key == "folders" && field.array.is_none()));
            self.fields[0].push(Field {
                section: "email",
                array: Some(("accounts", 0)),
                key: "folders",
                label: "Default account folders".into(),
                description: "Comma-separated IMAP folders; INBOX is the usual starting point.",
                value: folders
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                original: String::new(),
                integer: false,
                list: true,
                credential: None,
                secret: false,
            });
        }
        let Some(accounts) = self.doc["email"]["accounts"].as_array_of_tables_mut() else {
            self.error = Some("Email accounts are not a list".into());
            return;
        };
        let index = accounts.len();
        let mut label = format!("account{}", index + 1);
        while accounts
            .iter()
            .any(|a| a.get("label").and_then(Item::as_str) == Some(&label))
        {
            label.push('_');
        }
        let mut account = Table::new();
        account["label"] = toml_edit::value(label.as_str());
        account["folders"] = toml_edit::value(toml_edit::Array::from_iter(["INBOX"]));
        accounts.push(account);
        let credential_index = self.credentials.len();
        let credential_path = self
            .path
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""))
            .join("credentials")
            .join(format!("imap_{label}.toml"));
        let credential_doc = DocumentMut::new();
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
                "Email address used to sign in.",
                false,
                false,
            ),
            (
                "app_password",
                "App password",
                "Use an app-specific password from your mail provider.",
                true,
                false,
            ),
        ] {
            let value = if integer {
                "993".to_owned()
            } else {
                String::new()
            };
            self.fields[0].push(Field {
                section: "email",
                array: None,
                key,
                label: format!("{label} {name}"),
                description,
                original: value.clone(),
                value,
                integer,
                list: false,
                credential: Some(credential_index),
                secret,
            });
        }
        self.fields[0].push(Field {
            section: "email",
            array: Some(("accounts", index)),
            key: "folders",
            label: format!("{label} folders"),
            description: "Comma-separated IMAP folders; INBOX is the usual starting point.",
            value: "INBOX".into(),
            original: "INBOX".into(),
            integer: false,
            list: true,
            credential: None,
            secret: false,
        });
        self.credentials.push(Credential {
            kind: "imap",
            path: credential_path,
            original: None,
            doc: credential_doc,
        });
        self.row = self.fields[0].len() - 5;
    }

    fn remove_last_source(&mut self) {
        let (section, key, tab) = match self.tab {
            0 => ("email", "accounts", 0),
            1 => ("calendar", "providers", 1),
            2 => ("feeds", "feeds", 2),
            _ => return,
        };
        let index = self
            .doc
            .get(section)
            .and_then(|s| s.get(key))
            .and_then(Item::as_array_of_tables)
            .map(|a| a.len().saturating_sub(1));
        let Some(index) = index.filter(|_| {
            self.doc
                .get(section)
                .and_then(|s| s.get(key))
                .and_then(Item::as_array_of_tables)
                .is_some_and(|a| !a.is_empty())
        }) else {
            self.error = Some(format!(
                "No {} to remove",
                match tab {
                    0 => "email account",
                    1 => "calendar source",
                    _ => "feed",
                }
            ));
            return;
        };
        let label = (tab == 0)
            .then(|| {
                self.doc[section][key]
                    .as_array_of_tables()
                    .and_then(|a| a.get(index))
                    .and_then(|a| a.get("label"))
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .to_owned()
            })
            .unwrap_or_default();
        if let Some(array) = self.doc[section][key].as_array_of_tables_mut() {
            array.remove(index);
        }
        if tab == 0 {
            let filename = if label.is_empty() {
                "imap.toml".into()
            } else {
                format!("imap_{label}.toml")
            };
            let removed_path = self
                .path
                .parent()
                .unwrap_or_else(|| std::path::Path::new(""))
                .join("credentials")
                .join(filename);
            if let Some(credential) = self.credentials.iter().position(|c| c.path == removed_path) {
                self.credentials.remove(credential);
                self.fields[0].retain(|field| field.credential != Some(credential));
                for field in self.fields.iter_mut().flatten() {
                    if let Some(i) = field.credential {
                        if i > credential {
                            field.credential = Some(i - 1);
                        }
                    }
                }
            }
            self.fields[tab].retain(|f| {
                f.array != Some((key, index)) && !f.label.starts_with(&format!("{label} "))
            });
        } else {
            self.fields[tab].retain(|f| f.array != Some((key, index)));
        }
        self.row = self.row.min(self.fields[tab].len().saturating_sub(1));
    }

    fn herdr_row_count(&self) -> usize {
        self.herdr_bindings.len() + 2
    }

    fn dirty(&self) -> bool {
        self.doc.to_string() != self.original
            || self
                .fields
                .iter()
                .flatten()
                .any(|field| field.value != field.original)
            || self.herdr_bindings != herdr_bindings(&self.herdr_doc)
    }

    fn update_herdr_plugin(&mut self) {
        let status = Arc::clone(&self.herdr_update_status);
        if let Ok(mut text) = status.lock() {
            if text.as_str() == "Updating docket plugin…" {
                return;
            }
            *text = "Updating docket plugin…".into();
        }
        thread::spawn(move || {
            let result = Command::new("herdr")
                .args(["plugin", "update", "nicococo.docket"])
                .stdin(std::process::Stdio::null())
                .output();
            let message = match result {
                Ok(output) if output.status.success() => "Plugin updated. Reopen docket.".into(),
                Ok(output) => format!(
                    "Plugin update failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
                Err(error) => format!("Could not run Herdr: {error}"),
            };
            if let Ok(mut text) = status.lock() {
                *text = message;
            }
        });
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        self.error = None;
        if self.tab == 4 {
            return self.handle_herdr_key(key);
        }
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
            (_, KeyCode::Char('a' | 'n')) if !self.editing && self.tab == 0 => {
                self.add_email_account()
            }
            (_, KeyCode::Char('a' | 'n')) if !self.editing && self.tab == 2 => self.add_feed(),
            (_, KeyCode::Char('a' | 'n')) if !self.editing && self.tab == 1 => {
                self.add_calendar_source()
            }
            (_, KeyCode::Char('x')) if !self.editing && self.tab < 3 => self.remove_last_source(),
            (_, KeyCode::Up) if !self.editing => self.row = self.row.saturating_sub(1),
            (_, KeyCode::Down) if !self.editing => {
                self.row = (self.row + 1).min(self.fields[self.tab].len().saturating_sub(1))
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

    fn handle_herdr_key(&mut self, key: KeyEvent) -> bool {
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => match self.save() {
                Ok(()) => {
                    self.saved = true;
                    return true;
                }
                Err(error) => self.error = Some(format!("{error:#}")),
            },
            (_, KeyCode::Esc) if self.editing => self.editing = false,
            (_, KeyCode::Esc) => return true,
            (_, KeyCode::Tab) if !self.editing => {
                self.tab = (self.tab + 1) % TABS.len();
                self.row = 0;
            }
            (_, KeyCode::BackTab) if !self.editing => {
                self.tab = (self.tab + TABS.len() - 1) % TABS.len();
                self.row = 0;
            }
            (_, KeyCode::Up) if !self.editing => self.row = self.row.saturating_sub(1),
            (_, KeyCode::Down) if !self.editing => {
                self.row = (self.row + 1).min(self.herdr_row_count().saturating_sub(1))
            }
            (_, KeyCode::Char('x')) if !self.editing && self.row < self.herdr_bindings.len() => {
                self.herdr_bindings.remove(self.row);
                self.row = self.row.min(self.herdr_row_count().saturating_sub(1));
            }
            (_, KeyCode::Enter) if self.row == self.herdr_bindings.len() && !self.editing => {
                self.herdr_new_binding.clear();
                self.editing = true;
            }
            (_, KeyCode::Enter) if self.editing => {
                let key = self.herdr_new_binding.trim().to_owned();
                if key.is_empty() {
                    self.error = Some("Enter a Herdr key sequence".into());
                } else if self.herdr_bindings.contains(&key) {
                    self.error = Some("That docket shortcut is already listed".into());
                } else {
                    self.herdr_bindings.push(key);
                    self.row = self.herdr_bindings.len() - 1;
                    self.editing = false;
                }
            }
            (_, KeyCode::Enter) if self.row == self.herdr_bindings.len() + 1 => {
                self.update_herdr_plugin();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u')) if self.editing => {
                self.herdr_new_binding.clear();
            }
            (_, KeyCode::Backspace) if self.editing => {
                self.herdr_new_binding.pop();
            }
            (m, KeyCode::Char(c)) if self.editing && (m.is_empty() || m == KeyModifiers::SHIFT) => {
                if c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '_') {
                    self.herdr_new_binding.push(c);
                }
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
        let herdr_changed = self.herdr_bindings != herdr_bindings(&self.herdr_doc);
        let mut herdr_doc = self.herdr_doc.clone();
        if herdr_changed {
            let commands = herdr_doc
                .get("keys")
                .and_then(|keys| keys.get("command"))
                .and_then(Item::as_array_of_tables);
            for key in &self.herdr_bindings {
                if commands.is_some_and(|commands| {
                    commands.iter().any(|command| {
                        command.get("command").and_then(Item::as_str) != Some(HERDR_ACTION)
                            && command.get("key").and_then(Item::as_str) == Some(key)
                    })
                }) {
                    bail!("Herdr shortcut {key:?} is already assigned");
                }
            }
            if herdr_doc.get("keys").is_none() {
                herdr_doc["keys"] = Item::Table(Table::new());
            }
            if herdr_doc["keys"].get("command").is_none() {
                herdr_doc["keys"]["command"] = Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
            }
            let commands = herdr_doc["keys"]["command"]
                .as_array_of_tables_mut()
                .context("Herdr key commands are not a list")?;
            let mut index = 0;
            while index < commands.len() {
                if commands
                    .get(index)
                    .and_then(|command| command.get("command"))
                    .and_then(Item::as_str)
                    == Some(HERDR_ACTION)
                {
                    commands.remove(index);
                } else {
                    index += 1;
                }
            }
            for key in &self.herdr_bindings {
                let mut command = Table::new();
                command["key"] = toml_edit::value(key.as_str());
                command["type"] = toml_edit::value("plugin_action");
                command["command"] = toml_edit::value(HERDR_ACTION);
                command["description"] = toml_edit::value("Toggle docket");
                commands.push(command);
            }
        }
        if herdr_changed {
            let current = match std::fs::read_to_string(&self.herdr_path) {
                Ok(text) => Some(text),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            if current != self.herdr_original {
                bail!("Herdr settings changed on disk. Reopen before saving.");
            }
        }
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
        if herdr_changed {
            write_config_atomic(&self.herdr_path, &herdr_doc.to_string())?;
            self.herdr_original = Some(herdr_doc.to_string());
            self.herdr_doc = herdr_doc;
            let _ = thread::Builder::new()
                .name("docket-herdr-reload".into())
                .spawn(|| {
                    let _ = Command::new("herdr")
                        .args(["server", "reload-config"])
                        .stdin(std::process::Stdio::null())
                        .output();
                });
        }
        self.original = text;
        self.doc = doc;
        Ok(())
    }

    pub fn render(&self, frame: &mut Frame, theme: &crate::theme::Theme) {
        let area = frame.area();
        if area.width < 20 || area.height < 9 {
            return;
        }
        let herdr = self.tab == 4;
        let row_count = if herdr {
            self.herdr_row_count()
        } else {
            self.fields[self.tab].len()
        };
        let hint = if area.width < 58 && self.editing {
            "Edit · ^U clear · Enter done · ^S save"
        } else if area.width < 58 && !self.editing && !herdr {
            "↑↓ move · a add · x remove · Enter edit · ^S save"
        } else if self.editing {
            "Type · ^U clear · Enter done · ^S save"
        } else if herdr {
            "Enter action · x remove · ^S save · Esc close"
        } else if self.tab < 3 {
            "Tab pages · ↑/↓ move · Enter edit · a add · x remove last · ^S save"
        } else {
            "Tab pages · ↑/↓ move · Enter edit · ^S save · Esc close"
        };
        let footer = if self.dirty() {
            format!("{hint} · unsaved")
        } else {
            hint.to_owned()
        };
        let visible_rows = row_count
            .min(usize::from(area.height.saturating_sub(10)))
            .max(1);
        let label_width = if herdr {
            24
        } else {
            self.fields[self.tab]
                .iter()
                .map(|f| f.label.chars().count())
                .max()
                .unwrap_or(12)
                .min(24)
        };
        let value_width = if herdr {
            self.herdr_bindings
                .iter()
                .map(String::len)
                .max()
                .unwrap_or(12)
                .min(30)
        } else {
            self.fields[self.tab]
                .iter()
                .map(|f| f.value.chars().count())
                .max()
                .unwrap_or(12)
                .min(36)
        };
        let description_width = if herdr {
            72
        } else {
            self.fields[self.tab]
                .iter()
                .map(|field| field.description.chars().count())
                .max()
                .unwrap_or(0)
                .min(72)
        };
        let natural_width = label_width
            .saturating_add(value_width)
            .saturating_add(8)
            .max(description_width + 4)
            .max(TABS.iter().map(|tab| tab.len()).sum::<usize>() + TABS.len() * 2)
            .max(footer.chars().count() + 2)
            .max(46);
        let width = natural_width
            .min(usize::from(area.width.saturating_sub(2)))
            .max(16) as u16;
        let height = (visible_rows + 8)
            .min(usize::from(area.height.saturating_sub(2)))
            .max(9) as u16;
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(theme.border_focused)
            .title(Span::styled(
                if width < 38 {
                    " SETTINGS "
                } else {
                    " ✦ DOCKET SETTINGS ✦ "
                },
                theme.widget_title_focused,
            ));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let tab_labels = if inner.width < 28 {
            ["E", "C", "F", "N", "H"]
        } else if inner.width < 42 {
            ["Mail", "Cal", "Feed", "Notes", "Herdr"]
        } else {
            TABS
        };
        let tabs = Line::from(
            tab_labels
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    Span::styled(
                        format!(" {t} "),
                        if i == self.tab {
                            theme.widget_title_focused
                        } else {
                            theme.text_brilliant
                        },
                    )
                })
                .collect::<Vec<_>>(),
        );
        frame.render_widget(
            Paragraph::new(tabs),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        let h = inner.height.saturating_sub(6) as usize;
        let top = self.row.saturating_sub(h.saturating_sub(1));
        let lines = if herdr {
            let mut rows: Vec<(String, String)> = self
                .herdr_bindings
                .iter()
                .map(|key| ("Shortcut".into(), key.clone()))
                .collect();
            rows.push((
                "Add shortcut".into(),
                if self.editing {
                    self.herdr_new_binding.clone()
                } else {
                    "Enter to add".into()
                },
            ));
            rows.push(("Update plugin".into(), "Enter to update".into()));
            rows.into_iter()
                .enumerate()
                .skip(top)
                .take(h)
                .map(|(i, (label, value))| {
                    let row = format!(
                        "{} {label:<24} {value}",
                        if i == self.row { "▌" } else { " " }
                    );
                    Line::from(format!("{row:<width$}", width = inner.width as usize)).style(
                        if i == self.row {
                            Style::default()
                                .fg(Color::Black)
                                .bg(theme.text_selected.fg.unwrap_or(Color::Yellow))
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        },
                    )
                })
                .collect::<Vec<_>>()
        } else {
            self.fields[self.tab]
                .iter()
                .enumerate()
                .skip(top)
                .take(h)
                .map(|(i, f)| {
                    let style = if i == self.row {
                        Style::default()
                            .fg(Color::Black)
                            .bg(theme.text_selected.fg.unwrap_or(Color::Yellow))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    let row = format!(
                        "{} {:<width$} {}",
                        if i == self.row { "▌" } else { " " },
                        f.label,
                        if f.secret {
                            "*".repeat(f.value.chars().count())
                        } else if f.value.is_empty() {
                            "‹ empty ›".into()
                        } else {
                            f.value.clone()
                        },
                        width = label_width,
                    );
                    Line::from(format!("{row:<width$}", width = inner.width as usize)).style(style)
                })
                .collect::<Vec<_>>()
        };
        frame.render_widget(
            Paragraph::new(lines),
            Rect::new(inner.x, inner.y + 1, inner.width, h as u16),
        );
        let herdr_status = self
            .herdr_update_status
            .lock()
            .ok()
            .map(|s| s.clone())
            .unwrap_or_default();
        let herdr_desc = if self.row < self.herdr_bindings.len() {
            "Herdr toggle key. Use the full sequence, e.g. prefix+i; Herdr's prefix key is configured separately. Press x to remove."
        } else if self.row == self.herdr_bindings.len() {
            "Type the full Herdr sequence, e.g. prefix+i (the prefix key is set in Herdr config); save to apply."
        } else if herdr_status.is_empty() {
            "Update the installed plugin in the background; reopen after it finishes."
        } else {
            &herdr_status
        };
        let desc = self.error.as_deref().unwrap_or_else(|| {
            if herdr {
                herdr_desc
            } else {
                self.fields[self.tab]
                    .get(self.row)
                    .map(|f| f.description)
                    .unwrap_or("")
            }
        });
        let description = if self.error.is_some() {
            format!("! {desc}")
        } else {
            format!("↳ {desc}")
        };
        let desc_y = inner.y + inner.height.saturating_sub(4);
        frame.render_widget(
            Paragraph::new(description)
                .wrap(Wrap { trim: true })
                .style(if self.error.is_some() {
                    theme.text_shortcut
                } else {
                    theme.text_focused
                }),
            Rect::new(inner.x, desc_y, inner.width, 2),
        );
        frame.render_widget(
            Paragraph::new(footer).style(theme.text_selected),
            Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1),
        );
    }
}

fn herdr_bindings(doc: &DocumentMut) -> Vec<String> {
    doc.get("keys")
        .and_then(|keys| keys.get("command"))
        .and_then(Item::as_array_of_tables)
        .map(|commands| {
            commands
                .iter()
                .filter_map(|command| {
                    (command.get("command").and_then(Item::as_str) == Some(HERDR_ACTION)).then(
                        || {
                            command
                                .get("key")
                                .and_then(Item::as_str)
                                .unwrap_or("")
                                .to_owned()
                        },
                    )
                })
                .filter(|key| !key.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn write_config_atomic(path: &std::path::Path, text: &str) -> Result<()> {
    let parent = path.parent().context("Config path has no directory")?;
    std::fs::create_dir_all(parent)?;
    let tmp = path.with_extension(format!("toml.{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mode = std::fs::metadata(path)
            .map(|metadata| {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o777
            })
            .unwrap_or(0o600);
        options.mode(mode);
    }
    let mut file = options.open(&tmp)?;
    let result = (|| -> Result<()> {
        use std::io::Write;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
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

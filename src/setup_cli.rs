// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 nicococo

use std::{
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Item, Table};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Add or remove named RSS/Atom sources.
    Feed {
        #[command(subcommand)]
        command: FeedCommand,
    },
    /// Add or remove named IMAP accounts.
    Email {
        #[command(subcommand)]
        command: EmailCommand,
    },
    /// Add or remove named calendar sources.
    Calendar {
        #[command(subcommand)]
        command: CalendarCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum FeedCommand {
    /// Add a feed; NAME is shown as its tab label.
    Add { name: String, url: String },
    /// Remove the feed with this tab label.
    Remove { name: String },
}

#[derive(Debug, Subcommand)]
pub enum EmailCommand {
    /// Add an IMAP account. Pipe its app password to stdin.
    Add(EmailAdd),
    /// Remove the named account and its credential file.
    Remove { name: String },
}

#[derive(Debug, Args)]
pub struct EmailAdd {
    pub name: String,
    #[arg(long)]
    pub host: String,
    #[arg(long)]
    pub username: String,
    #[arg(long, default_value_t = 993)]
    pub port: u16,
    #[arg(long = "folder", default_value = "INBOX")]
    pub folders: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum CalendarCommand {
    /// Add a local, CalDAV, or ICS source. For ICS, pipe its URL to stdin.
    Add(CalendarAdd),
    /// Remove a source by its account name.
    Remove { name: String },
}

#[derive(Debug, Args)]
pub struct CalendarAdd {
    pub name: String,
    /// local, caldav, or ics
    #[arg(long, value_parser = ["local", "caldav", "ics"])]
    pub kind: String,
    /// CalDAV server URL (required with --kind caldav).
    #[arg(long)]
    pub server: Option<String>,
    /// CalDAV username (required with --kind caldav).
    #[arg(long)]
    pub username: Option<String>,
    /// Restrict CalDAV to these calendar collection URLs (repeatable).
    #[arg(long = "calendar-id")]
    pub calendar_ids: Vec<String>,
}

pub fn run(command: Command, config: Option<&Path>) -> Result<()> {
    let path = config
        .map(Path::to_path_buf)
        .unwrap_or(crate::config::config_path()?);
    let original = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "Could not read {}. Run `docket --init` first.",
            path.display()
        )
    })?;
    let mut doc: DocumentMut = original.parse().context("Invalid configuration")?;
    let credentials_dir = path.parent().unwrap_or(Path::new(".")).join("credentials");
    let mut credential_write: Option<(PathBuf, String)> = None;
    let mut credential_remove: Option<PathBuf> = None;
    let message = match command {
        Command::Feed { command } => match command {
            FeedCommand::Add { name, url } => {
                valid_name(&name)?;
                valid_url(&url)?;
                let feeds = array_table(&mut doc, "feeds", "feeds");
                if feeds
                    .iter()
                    .any(|t| t.get("topic").and_then(Item::as_str) == Some(&name))
                {
                    bail!("feed already exists: {name}");
                }
                let mut t = Table::new();
                t["topic"] = value(name.clone());
                t["url"] = value(url);
                feeds.push(t);
                format!("Added feed {name}")
            }
            FeedCommand::Remove { name } => {
                let feeds = array_table(&mut doc, "feeds", "feeds");
                remove_one(
                    feeds,
                    |t| t.get("topic").and_then(Item::as_str) == Some(&name),
                    "feed",
                    &name,
                )?;
                format!("Removed feed {name}")
            }
        },
        Command::Email { command } => match command {
            EmailCommand::Add(args) => {
                valid_name(&args.name)?;
                if args.port == 0 {
                    bail!("port must be between 1 and 65535");
                }
                if args.host.trim().is_empty() || args.username.trim().is_empty() {
                    bail!("host and username are required");
                }
                let mut secret = stdin_secret("app password")?;
                if secret.is_empty() {
                    bail!("app password cannot be empty");
                }
                let accounts = array_table(&mut doc, "email", "accounts");
                if accounts
                    .iter()
                    .any(|t| t.get("label").and_then(Item::as_str) == Some(&args.name))
                {
                    bail!("email account already exists: {}", args.name);
                }
                let mut folders = Array::new();
                for f in &args.folders {
                    folders.push(f.as_str());
                }
                let mut t = Table::new();
                t["label"] = value(args.name.clone());
                t["folders"] = Item::Value(folders.into());
                accounts.push(t);
                let path = credentials_dir.join(format!("imap_{}.toml", args.name));
                if path.exists() {
                    bail!("credential file already exists: {}", path.display());
                }
                let cred = format!(
                    "host = {}\nport = {}\nuse_tls = true\nusername = {}\napp_password = {}\n",
                    toml_edit::ser::to_string(&args.host)?,
                    args.port,
                    toml_edit::ser::to_string(&args.username)?,
                    toml_edit::ser::to_string(&secret)?
                );
                secret.clear();
                credential_write = Some((path, cred));
                format!("Added email account {}", args.name)
            }
            EmailCommand::Remove { name } => {
                valid_name(&name)?;
                let accounts = array_table(&mut doc, "email", "accounts");
                remove_one(
                    accounts,
                    |t| t.get("label").and_then(Item::as_str) == Some(&name),
                    "email account",
                    &name,
                )?;
                credential_remove = Some(credentials_dir.join(format!("imap_{name}.toml")));
                format!("Removed email account {name}")
            }
        },
        Command::Calendar { command } => match command {
            CalendarCommand::Add(args) => {
                valid_name(&args.name)?;
                let providers = array_table(&mut doc, "calendar", "providers");
                if providers
                    .iter()
                    .any(|t| t.get("account").and_then(Item::as_str) == Some(&args.name))
                {
                    bail!("calendar source already exists: {}", args.name);
                }
                let mut t = Table::new();
                t["kind"] = value(args.kind.clone());
                t["account"] = value(args.name.clone());
                if !args.calendar_ids.is_empty() {
                    let mut a = Array::new();
                    for id in &args.calendar_ids {
                        valid_url(id)?;
                        a.push(id.as_str());
                    }
                    t["calendar_ids"] = Item::Value(a.into());
                }
                if args.kind == "ics" {
                    let mut url = stdin_secret("ICS calendar URL")?;
                    valid_url(&url)?;
                    let p = credentials_dir.join("ics.toml");
                    let raw = std::fs::read_to_string(&p).unwrap_or_else(|_| "".into());
                    let mut c: DocumentMut = if raw.trim().is_empty() {
                        DocumentMut::new()
                    } else {
                        raw.parse().context("Invalid credentials/ics.toml")?
                    };
                    let feeds = array_table(&mut c, "", "feeds");
                    if feeds
                        .iter()
                        .any(|f| f.get("label").and_then(Item::as_str) == Some(&args.name))
                    {
                        bail!("ICS feed label already exists: {}", args.name);
                    }
                    let mut f = Table::new();
                    f["label"] = value(args.name.clone());
                    f["url"] = value(url.clone());
                    feeds.push(f);
                    url.clear();
                    credential_write = Some((p, c.to_string()));
                } else if args.kind == "caldav" {
                    let server = args
                        .server
                        .as_deref()
                        .context("--server is required for CalDAV")?;
                    valid_url(server)?;
                    let username = args
                        .username
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                        .context("--username is required for CalDAV")?;
                    let mut password = stdin_secret("CalDAV app password")?;
                    let p = credentials_dir.join(format!("caldav_{}.toml", args.name));
                    if p.exists() {
                        bail!("credential file already exists: {}", p.display());
                    }
                    let body = format!(
                        "server = {}\nusername = {}\napp_password = {}\n",
                        toml_edit::ser::to_string(server)?,
                        toml_edit::ser::to_string(username)?,
                        toml_edit::ser::to_string(&password)?
                    );
                    password.clear();
                    credential_write = Some((p, body));
                }
                providers.push(t);
                format!("Added {} calendar source {}", args.kind, args.name)
            }
            CalendarCommand::Remove { name } => {
                valid_name(&name)?;
                let providers = array_table(&mut doc, "calendar", "providers");
                let is_ics = providers
                    .iter()
                    .find(|t| t.get("account").and_then(Item::as_str) == Some(&name))
                    .and_then(|t| t.get("kind"))
                    .and_then(Item::as_str)
                    == Some("ics");
                let is_caldav = providers
                    .iter()
                    .find(|t| t.get("account").and_then(Item::as_str) == Some(&name))
                    .and_then(|t| t.get("kind"))
                    .and_then(Item::as_str)
                    == Some("caldav");
                remove_one(
                    providers,
                    |t| t.get("account").and_then(Item::as_str) == Some(&name),
                    "calendar source",
                    &name,
                )?;
                if is_ics {
                    let p = credentials_dir.join("ics.toml");
                    if p.exists() {
                        let raw = std::fs::read_to_string(&p)?;
                        let mut c: DocumentMut =
                            raw.parse().context("Invalid credentials/ics.toml")?;
                        let feeds = array_table(&mut c, "", "feeds");
                        let _ = remove_one(
                            feeds,
                            |f| f.get("label").and_then(Item::as_str) == Some(&name),
                            "ICS feed",
                            &name,
                        );
                        credential_write = Some((p, c.to_string()));
                    }
                }
                if is_caldav {
                    credential_remove = Some(credentials_dir.join(format!("caldav_{name}.toml")));
                }
                format!("Removed calendar source {name}")
            }
        },
    };
    if std::fs::read_to_string(&path)? != original {
        bail!("Configuration changed on disk; retry the command.");
    }
    if let Some((p, body)) = credential_write {
        write_private(&p, &body)?;
    }
    write_atomic(&path, &doc.to_string())?;
    if let Some(p) = credential_remove {
        let _ = std::fs::remove_file(p);
    }
    println!("{message}");
    Ok(())
}

fn valid_name(s: &str) -> Result<()> {
    if s.is_empty()
        || s.len() > 64
        || !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("name must be 1–64 letters, digits, underscores or hyphens");
    }
    Ok(())
}
fn valid_url(s: &str) -> Result<()> {
    let u = reqwest::Url::parse(s).context("invalid URL")?;
    if !matches!(u.scheme(), "http" | "https") {
        bail!("URL must use http or https");
    }
    Ok(())
}
fn stdin_secret(label: &str) -> Result<String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s)?;
    let s = s.trim().to_owned();
    if s.is_empty() {
        bail!("pipe {label} to stdin");
    }
    Ok(s)
}
fn array_table<'a>(doc: &'a mut DocumentMut, section: &str, key: &str) -> &'a mut ArrayOfTables {
    if section.is_empty() {
        if !doc.as_table().contains_key(key) {
            doc[key] = Item::ArrayOfTables(ArrayOfTables::new());
        }
        return doc[key].as_array_of_tables_mut().expect("array table");
    }
    if !doc.as_table().contains_key(section) {
        doc[section] = Item::Table(Table::new());
    }
    if !doc[section].as_table().expect("table").contains_key(key) {
        doc[section][key] = Item::ArrayOfTables(ArrayOfTables::new());
    }
    doc[section][key]
        .as_array_of_tables_mut()
        .expect("array table")
}
fn remove_one<F: Fn(&Table) -> bool>(
    rows: &mut ArrayOfTables,
    pred: F,
    kind: &str,
    name: &str,
) -> Result<()> {
    let ix: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, t)| pred(t).then_some(i))
        .collect();
    if ix.len() != 1 {
        bail!("expected one {kind} named {name}, found {}", ix.len());
    }
    rows.remove(ix[0]);
    Ok(())
}
fn write_private(path: &Path, body: &str) -> Result<()> {
    let parent = path.parent().context("credential path has no parent")?;
    std::fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    write_atomic(path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
fn write_atomic(path: &Path, body: &str) -> Result<()> {
    let parent = path.parent().context("path has no parent")?;
    std::fs::create_dir_all(parent)?;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mode = std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0o600);
        options.mode(mode);
    }
    let mut file = options
        .open(&tmp)
        .with_context(|| format!("creating {}", tmp.display()))?;
    use std::io::Write;
    file.write_all(body.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp, path).with_context(|| format!("updating {}", path.display()))?;
    Ok(())
}

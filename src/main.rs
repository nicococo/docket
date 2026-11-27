// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ntrospect0
// Copyright (C) 2026 nicococo

#[cfg(feature = "widget-notes")]
use std::io::Read;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};

use crate::widgets::parse_widget_ref;

mod app;
mod cache;
mod clipboard;
mod config;
mod credentials;
mod event;
mod format;
mod http;
mod pane_layout;
mod polling;
mod runtime_state;
mod settings;
mod setup_cli;
mod text;
mod theme;
mod ui;
mod widgets;
mod zoom;

/// docket — terminal dashboard for calendar, news, email, and getting things done.
#[derive(Parser, Debug)]
#[command(name = "docket", version, about, long_about = None)]
struct Cli {
    /// Create ~/.config/docket/ and seed default config files, then exit.
    /// Idempotent — existing files are left untouched, so it's also the
    /// right thing to run after upgrading to pick up any new default files.
    #[arg(long)]
    init: bool,

    /// Clear cached data before launching. With no value, wipes
    /// `$XDG_CACHE_HOME/docket/` entirely. Pass a widget kind (`news`) or
    /// `kind@instance` (`news@home`) to scope the clear. Prompts for [y/N]
    /// confirmation; use --clear-cache-forced to skip the prompt.
    #[arg(long, value_name = "TARGET", num_args = 0..=1, default_missing_value = "*")]
    clear_cache: Option<String>,

    /// Like --clear-cache but skips the confirmation prompt.
    #[arg(long, value_name = "TARGET", num_args = 0..=1, default_missing_value = "*")]
    clear_cache_forced: Option<String>,

    /// Path to a config file (overrides the default XDG location).
    #[arg(long, value_name = "FILE", global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Read and write shared Markdown notes from an agent or script.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Add or remove named feeds, email accounts, and calendar sources.
    Setup {
        #[command(subcommand)]
        command: setup_cli::Command,
    },
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    /// Work with docket's shared Markdown notes.
    Notes {
        #[command(subcommand)]
        command: AgentNotesCommand,
    },
    /// Read and maintain scoped Markdown memory for agents.
    Memory {
        #[command(subcommand)]
        command: AgentMemoryCommand,
    },
    /// Read the cached calendar timeline.
    Calendar {
        #[command(subcommand)]
        command: ListCommand,
    },
    /// Read cached RSS and Atom articles.
    Feeds {
        #[command(subcommand)]
        command: ListCommand,
    },
    /// Read cached mailbox messages.
    Email {
        #[command(subcommand)]
        command: EmailCommand,
    },
}

#[derive(Debug, Subcommand)]
enum AgentNotesCommand {
    /// List notes as JSON, newest first.
    List,
    /// Print one Markdown note by its note ID.
    Read { id: String },
    /// Add a Markdown note read from stdin; its first line is the title.
    Add,
}

#[derive(Debug, Subcommand)]
enum AgentMemoryCommand {
    /// Print the resolved project, role, agent ID, and note path.
    #[command(alias = "identify", alias = "whoami")]
    Id(MemoryScope),
    /// List this agent's Markdown memories as JSON.
    List(MemoryScope),
    /// Print a memory by its ID from this scope.
    Read {
        id: String,
        #[command(flatten)]
        scope: MemoryScope,
    },
    /// Add Markdown from stdin; its first line is the note title.
    Add(MemoryScope),
    /// Replace a memory with Markdown from stdin.
    Edit {
        id: String,
        #[command(flatten)]
        scope: MemoryScope,
    },
    /// Delete a memory from this scope.
    Delete {
        id: String,
        #[command(flatten)]
        scope: MemoryScope,
    },
}

#[derive(Debug, clap::Args, Clone)]
struct MemoryScope {
    /// Project scope (defaults to DOCKET_PROJECT or the current Git root).
    #[arg(long)]
    project: Option<String>,
    /// Logical role such as planner, implementer, or reviewer.
    #[arg(long)]
    role: Option<String>,
    /// Stable agent label such as codex, omp, or deepseek.
    #[arg(long)]
    agent: Option<String>,
}

#[derive(Debug, Subcommand)]
enum ListCommand {
    /// Print the cached data as JSON.
    List,
}

#[derive(Debug, Subcommand)]
enum EmailCommand {
    /// Print cached message headers as JSON.
    List,
    /// Print the cached plain-text body for a message ID.
    Read { id: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(path) = cli.config.as_deref() {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            config::set_config_dir_override(parent.to_path_buf());
        }
    }

    init_tracing();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async move {
        if cli.init {
            let path = config::init_default_config()?;
            println!("Initialized config at {}", path.display());
            return Ok(());
        }
        // --clear-cache / --clear-cache-forced fire before the rest of startup
        // so a confirmed clear flows straight into the dashboard with a fresh
        // slate. Conflicting flags are flagged early.
        if cli.clear_cache.is_some() && cli.clear_cache_forced.is_some() {
            return Err(anyhow!(
                "pass either --clear-cache or --clear-cache-forced, not both"
            ));
        }
        if let Some(target) = cli.clear_cache.as_deref() {
            run_clear_cache(target, true)?;
        }
        if let Some(target) = cli.clear_cache_forced.as_deref() {
            run_clear_cache(target, false)?;
        }
        // First-run UX: seed default config files before opening the TUI.
        // `--config <path>` opts out — the user explicitly named a file.
        if cli.config.is_none() && !looks_initialized() {
            eprintln!("No config detected at ~/.config/docket/config.toml — writing defaults.");
            eprintln!(
                "Edit that file (or the per-widget TOMLs alongside it) to customize your dashboard."
            );
            eprintln!();
            config::init_default_config()?;
        }

        if let Some(command) = cli.command {
            return match command {
                Command::Agent { .. } => run_agent_command(command, cli.config.as_deref()),
                Command::Setup { command } => setup_cli::run(command, cli.config.as_deref()),
            };
        }

        app::run(cli.config).await
    })
}

fn run_agent_command(command: Command, config_path: Option<&std::path::Path>) -> Result<()> {
    let Command::Agent { command } = command else {
        anyhow::bail!("expected an agent command");
    };
    match command {
        AgentCommand::Notes { command } => run_agent_notes(command, config_path),
        AgentCommand::Memory { command } => run_agent_memory(command, config_path),
        AgentCommand::Calendar { command } => {
            #[cfg(feature = "widget-calendar")]
            return run_agent_calendar(command);
            #[cfg(not(feature = "widget-calendar"))]
            anyhow::bail!("agent calendar requires building with the widget-calendar feature")
        }
        AgentCommand::Feeds { command } => {
            #[cfg(feature = "widget-feeds")]
            return run_agent_feeds(command);
            #[cfg(not(feature = "widget-feeds"))]
            anyhow::bail!("agent feeds requires building with the widget-feeds feature")
        }
        AgentCommand::Email { command } => {
            #[cfg(feature = "widget-email")]
            return run_agent_email(command);
            #[cfg(not(feature = "widget-email"))]
            anyhow::bail!("agent email requires building with the widget-email feature")
        }
    }
}

#[cfg(feature = "widget-notes")]
fn run_agent_memory(
    command: AgentMemoryCommand,
    config_path: Option<&std::path::Path>,
) -> Result<()> {
    use widgets::notes::store::{self, Note};

    let config = config::load(config_path)?;
    let (root, _) = store::resolve_root(config.notes.notes_dir.as_deref())?;
    let scope = match &command {
        AgentMemoryCommand::Id(s) | AgentMemoryCommand::List(s) | AgentMemoryCommand::Add(s) => {
            s.clone()
        }
        AgentMemoryCommand::Read { scope, .. }
        | AgentMemoryCommand::Edit { scope, .. }
        | AgentMemoryCommand::Delete { scope, .. } => scope.clone(),
    }
    .resolve()?;
    let prefix = scope.prefix();
    let notes = store::load_all(&root, "main");
    let in_scope = |note: &&Note| note.display_name().starts_with(&format!("{prefix}>"));

    match command {
        AgentMemoryCommand::Id(_) => println!(
            "{}",
            serde_json::json!({
                "id": format!("{}/{}/{}", scope.project, scope.role, scope.agent),
                "project": scope.project,
                "role": scope.role,
                "path": prefix,
                "notes_dir": root,
            })
        ),
        AgentMemoryCommand::List(_) => println!(
            "{}",
            serde_json::Value::Array(
                notes
                    .iter()
                    .filter(in_scope)
                    .map(|note| serde_json::json!({
                        "id": note.id,
                        "title": note.display_name().strip_prefix(&format!("{prefix}>" )).unwrap_or(note.display_name()),
                        "path": note.display_name(),
                        "modified": note.modified.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
                    }))
                    .collect(),
            )
        ),
        AgentMemoryCommand::Read { id, .. } => {
            print!("{}", scoped_note_body(&notes, &id, &prefix)?);
        }
        AgentMemoryCommand::Add(_) => {
            let mut body = String::new();
            io::stdin().read_to_string(&mut body)?;
            let mut note = Note {
                id: String::new(),
                body: memory_body(&body, &prefix)?,
                modified: std::time::SystemTime::UNIX_EPOCH,
            };
            store::save(&root, "main", &mut note)?;
            println!("{}", serde_json::json!({ "id": note.id, "path": note.display_name() }));
        }
        AgentMemoryCommand::Edit { id, .. } => {
            let old = notes
                .iter()
                .find(|note| note.id == id && note.display_name().starts_with(&format!("{prefix}>")))
                .with_context(|| format!("memory not found in this scope: {id}"))?;
            let mut body = String::new();
            io::stdin().read_to_string(&mut body)?;
            let mut note = Note {
                id: old.id.clone(),
                body: memory_body(&body, &prefix)?,
                modified: old.modified,
            };
            store::save(&root, "main", &mut note)?;
            println!("{}", serde_json::json!({ "id": note.id, "path": note.display_name() }));
        }
        AgentMemoryCommand::Delete { id, .. } => {
            let note = notes
                .iter()
                .find(|note| note.id == id && note.display_name().starts_with(&format!("{prefix}>")))
                .with_context(|| format!("memory not found in this scope: {id}"))?;
            store::delete(&root, "main", &note.id)?;
            println!("{}", serde_json::json!({ "deleted": note.id }));
        }
    }
    Ok(())
}

#[cfg(not(feature = "widget-notes"))]
fn run_agent_memory(
    command: AgentMemoryCommand,
    config_path: Option<&std::path::Path>,
) -> Result<()> {
    let _ = (command, config_path);
    anyhow::bail!("agent memory requires building with the widget-notes feature")
}

#[derive(Debug, Clone)]
struct ResolvedMemoryScope {
    project: String,
    role: String,
    agent: String,
}

impl ResolvedMemoryScope {
    fn prefix(&self) -> String {
        format!("Agents>{}>{}>{}", self.project, self.role, self.agent)
    }
}

impl MemoryScope {
    fn resolve(self) -> Result<ResolvedMemoryScope> {
        let project = self
            .project
            .or_else(|| std::env::var("DOCKET_PROJECT").ok())
            .unwrap_or_else(default_project_name);
        let role = self
            .role
            .or_else(|| std::env::var("DOCKET_AGENT_ROLE").ok())
            .unwrap_or_else(|| "general".into());
        let agent = self
            .agent
            .or_else(|| std::env::var("DOCKET_AGENT_ID").ok())
            .unwrap_or_else(|| "shared".into());
        for (label, value) in [("project", &project), ("role", &role), ("agent", &agent)] {
            if value.is_empty()
                || value.len() > 64
                || !value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
            {
                anyhow::bail!("{label} must be 1–64 letters, digits, dots, underscores or hyphens");
            }
        }
        Ok(ResolvedMemoryScope {
            project,
            role,
            agent,
        })
    }
}

fn default_project_name() -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    let root = cwd
        .ancestors()
        .find(|path| path.join(".git").exists())
        .unwrap_or(&cwd);
    root.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("default")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

#[cfg(feature = "widget-notes")]
fn memory_body(body: &str, prefix: &str) -> Result<String> {
    let mut lines = body.lines();
    let title = lines.next().unwrap_or_default().trim();
    if title.is_empty() {
        anyhow::bail!("pipe Markdown with a title on the first line to stdin");
    }
    let rest = lines.collect::<Vec<_>>().join("\n");
    Ok(if rest.is_empty() {
        format!("{prefix}>{title}")
    } else {
        format!("{prefix}>{title}\n{rest}")
    })
}

#[cfg(feature = "widget-notes")]
fn scoped_note_body<'a>(
    notes: &'a [widgets::notes::store::Note],
    id: &str,
    prefix: &str,
) -> Result<&'a str> {
    notes
        .iter()
        .find(|note| note.id == id && note.display_name().starts_with(&format!("{prefix}>")))
        .map(|note| note.body.as_str())
        .with_context(|| format!("memory not found in this scope: {id}"))
}

#[cfg(feature = "widget-notes")]
fn run_agent_notes(
    command: AgentNotesCommand,
    config_path: Option<&std::path::Path>,
) -> Result<()> {
    let config = config::load(config_path)?;
    let (root, _) = widgets::notes::store::resolve_root(config.notes.notes_dir.as_deref())?;
    let notes = widgets::notes::store::load_all(&root, "main");

    match command {
        AgentNotesCommand::List => {
            println!("{}", agent_notes_json(&notes));
        }
        AgentNotesCommand::Read { id } => {
            print!("{}", agent_note_body(&notes, &id)?);
        }
        AgentNotesCommand::Add => {
            let mut body = String::new();
            io::stdin().read_to_string(&mut body)?;
            println!(
                "{}",
                serde_json::json!({ "id": add_agent_note(&root, &body)? })
            );
        }
    }
    Ok(())
}

#[cfg(feature = "widget-notes")]
fn agent_notes_json(notes: &[widgets::notes::store::Note]) -> serde_json::Value {
    serde_json::Value::Array(
        notes
            .iter()
            .map(|note| {
                serde_json::json!({
                    "id": note.id,
                    "title": note.display_name(),
                    "modified": note.modified.duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default().as_secs(),
                })
            })
            .collect(),
    )
}

#[cfg(feature = "widget-notes")]
fn agent_note_body<'a>(notes: &'a [widgets::notes::store::Note], id: &str) -> Result<&'a str> {
    notes
        .iter()
        .find(|note| note.id == id)
        .map(|note| note.body.as_str())
        .with_context(|| format!("note not found: {id}"))
}

#[cfg(feature = "widget-notes")]
fn add_agent_note(root: &std::path::Path, body: &str) -> Result<String> {
    if body.trim().is_empty() {
        anyhow::bail!("pipe Markdown note content to stdin");
    }
    let mut note = widgets::notes::store::Note {
        id: String::new(),
        body: body.to_owned(),
        modified: std::time::SystemTime::UNIX_EPOCH,
    };
    widgets::notes::store::save(root, "main", &mut note)?;
    Ok(note.id)
}

#[cfg(not(feature = "widget-notes"))]
fn run_agent_notes(
    command: AgentNotesCommand,
    config_path: Option<&std::path::Path>,
) -> Result<()> {
    let _ = (command, config_path);
    anyhow::bail!("agent notes require building with the widget-notes feature")
}

#[cfg(feature = "widget-calendar")]
fn run_agent_calendar(command: ListCommand) -> Result<()> {
    let ListCommand::List = command;
    let cache = cache::Cache::open_default()?;
    println!(
        "{}",
        cached_collection_json::<widgets::calendar::provider::Event>(
            &cache, "calendar", "main", "events", "events"
        )
    );
    Ok(())
}

#[cfg(not(feature = "widget-calendar"))]
fn run_agent_calendar(command: ListCommand) -> Result<()> {
    let _ = command;
    anyhow::bail!("agent calendar requires building with the widget-calendar feature")
}

#[cfg(feature = "widget-feeds")]
fn run_agent_feeds(command: ListCommand) -> Result<()> {
    let ListCommand::List = command;
    let cache = cache::Cache::open_default()?;
    println!(
        "{}",
        cached_collection_json::<widgets::feeds::provider::FeedArticle>(
            &cache, "feeds", "ai", "articles", "articles"
        )
    );
    Ok(())
}

#[cfg(not(feature = "widget-feeds"))]
fn run_agent_feeds(command: ListCommand) -> Result<()> {
    let _ = command;
    anyhow::bail!("agent feeds requires building with the widget-feeds feature")
}

#[cfg(feature = "widget-email")]
fn run_agent_email(command: EmailCommand) -> Result<()> {
    let cache = cache::Cache::open_default()?;
    match command {
        EmailCommand::List => println!("{}", email_list_json(&cache)),
        EmailCommand::Read { id } => {
            print!("{}", email_body(&cache, &id)?);
        }
    }
    Ok(())
}

#[cfg(feature = "widget-email")]
fn email_list_json(cache: &cache::Cache) -> serde_json::Value {
    use widgets::email::provider::EmailMessage;

    let result = cache
        .scoped("email", "main")
        .load::<Vec<EmailMessage>>("messages")
        .map(|entry| {
            let messages: Vec<_> = entry
                .value
                .iter()
                .map(|message| {
                    serde_json::json!({
                        "id": message.id,
                        "folder": message.folder,
                        "account": message.account,
                        "from_name": message.from_name,
                        "from_address": message.from_address,
                        "subject": message.subject,
                        "received": message.received,
                        "server_unread": message.server_unread,
                    })
                })
                .collect();
            serde_json::json!({ "updated_at": entry.stored_at, "messages": messages })
        });
    result.unwrap_or_else(|| serde_json::json!({ "updated_at": null, "messages": [] }))
}

#[cfg(feature = "widget-email")]
fn email_body(cache: &cache::Cache, id: &str) -> Result<String> {
    use widgets::email::provider::EmailMessage;

    cache
        .scoped("email", "main")
        .load::<Vec<EmailMessage>>("messages")
        .and_then(|entry| entry.value.into_iter().find(|message| message.id == id))
        .map(|message| message.plain_body)
        .with_context(|| format!("cached email not found: {id}"))
}

fn cached_collection_json<T>(
    cache: &cache::Cache,
    kind: &str,
    instance: &str,
    key: &str,
    collection: &str,
) -> serde_json::Value
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let entry = cache.scoped(kind, instance).load::<Vec<T>>(key);
    let mut result = serde_json::Map::new();
    let (updated_at, values) = match entry {
        Some(entry) => (
            serde_json::json!(entry.stored_at),
            serde_json::json!(entry.value),
        ),
        None => (serde_json::Value::Null, serde_json::json!([])),
    };
    result.insert("updated_at".into(), updated_at);
    result.insert(collection.into(), values);
    serde_json::Value::Object(result)
}

#[cfg(test)]
mod agent_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_root(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "docket-agent-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn agent_commands_parse_with_config_at_the_end() {
        for (args, expected) in [
            (vec!["agent", "calendar", "list"], "calendar"),
            (vec!["agent", "feeds", "list"], "feeds"),
            (vec!["agent", "email", "list"], "email"),
            (vec!["agent", "email", "read", "imap-INBOX-4"], "email"),
            (vec!["agent", "notes", "add"], "notes"),
        ] {
            let cli = Cli::try_parse_from(
                std::iter::once("docket")
                    .chain(args.iter().copied())
                    .chain(["--config", "/tmp/docket-agent.toml"]),
            )
            .unwrap();
            assert_eq!(
                cli.config.as_deref(),
                Some(std::path::Path::new("/tmp/docket-agent.toml"))
            );
            let Some(Command::Agent { command }) = cli.command else {
                panic!("expected an agent command");
            };
            let actual = match command {
                AgentCommand::Calendar { .. } => "calendar",
                AgentCommand::Feeds { .. } => "feeds",
                AgentCommand::Email { .. } => "email",
                AgentCommand::Notes { .. } => "notes",
            };
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn cached_collections_report_empty_and_scoped_snapshots() {
        let root = temp_root("cache");
        let cache = cache::Cache::at(&root);
        let empty = cached_collection_json::<String>(&cache, "feeds", "ai", "articles", "articles");
        assert!(empty["updated_at"].is_null());
        assert_eq!(empty["articles"], serde_json::json!([]));

        cache
            .scoped("feeds", "ai")
            .store("articles", &vec!["one", "two"])
            .unwrap();
        cache
            .scoped("feeds", "work")
            .store("articles", &vec!["other instance"])
            .unwrap();
        let populated =
            cached_collection_json::<String>(&cache, "feeds", "ai", "articles", "articles");
        assert!(populated["updated_at"].is_string());
        assert_eq!(populated["articles"], serde_json::json!(["one", "two"]));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(feature = "widget-calendar", feature = "widget-feeds"))]
    #[test]
    fn calendar_and_feed_snapshots_use_the_widget_cache_shapes() {
        use chrono::{Duration, Local, Utc};
        use widgets::{calendar::provider::Event, feeds::provider::FeedArticle};

        let root = temp_root("snapshots");
        let cache = cache::Cache::at(&root);
        let start = Local::now();
        cache
            .scoped("calendar", "main")
            .store(
                "events",
                &[Event {
                    title: "Standup".into(),
                    start,
                    end: start + Duration::minutes(30),
                    all_day: false,
                    source: "local".into(),
                    calendar: "Personal".into(),
                    location: Some("Desk".into()),
                }],
            )
            .unwrap();
        cache
            .scoped("feeds", "ai")
            .store(
                "articles",
                &[FeedArticle {
                    title: "Release notes".into(),
                    url: "https://example.com/release".into(),
                    topic: "Tech".into(),
                    source: "Example".into(),
                    published: Utc::now(),
                    summary: Some("A new release".into()),
                    authors: vec!["Editor".into()],
                }],
            )
            .unwrap();

        let events =
            cached_collection_json::<Event>(&cache, "calendar", "main", "events", "events");
        assert_eq!(events["events"][0]["title"], "Standup");
        let articles =
            cached_collection_json::<FeedArticle>(&cache, "feeds", "ai", "articles", "articles");
        assert_eq!(articles["articles"][0]["title"], "Release notes");
        assert_eq!(articles["articles"][0]["authors"][0], "Editor");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "widget-notes")]
    #[test]
    fn agent_notes_add_list_and_read_share_the_markdown_store() {
        let root = temp_root("notes");
        let first = add_agent_note(&root, "First thought\n\nA quick note.").unwrap();
        let second = add_agent_note(&root, "First thought\n\nA follow-up.").unwrap();
        assert_eq!(first, "First thought");
        assert_eq!(second, "First thought 2");

        let notes = widgets::notes::store::load_all(&root, "main");
        assert_eq!(
            agent_note_body(&notes, &first).unwrap(),
            "First thought\n\nA quick note."
        );
        assert_eq!(
            agent_note_body(&notes, "missing").unwrap_err().to_string(),
            "note not found: missing"
        );
        let listed = agent_notes_json(&notes);
        assert_eq!(listed.as_array().unwrap().len(), 2);
        assert!(listed
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note["id"] == second));
        assert!(add_agent_note(&root, " \n ").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "widget-email")]
    #[test]
    fn email_list_hides_bodies_and_read_requires_a_matching_id() {
        use widgets::email::provider::EmailMessage;

        let root = temp_root("email");
        let cache = cache::Cache::at(&root);
        let message = EmailMessage {
            id: "imap-INBOX-4".into(),
            folder: "INBOX".into(),
            from_name: Some("Agent".into()),
            from_address: "agent@example.com".into(),
            subject: "Shared update".into(),
            received: chrono::Local::now(),
            server_unread: true,
            plain_body: "private body text".into(),
            account: "work".into(),
            imap_uid: Some(4),
        };
        cache
            .scoped("email", "main")
            .store("messages", &vec![message])
            .unwrap();

        let listing = email_list_json(&cache);
        assert_eq!(listing["messages"][0]["id"], "imap-INBOX-4");
        assert_eq!(listing["messages"][0]["subject"], "Shared update");
        assert_eq!(listing["messages"][0]["server_unread"], true);
        assert!(listing["messages"][0].get("plain_body").is_none());
        assert_eq!(
            email_body(&cache, "imap-INBOX-4").unwrap(),
            "private body text"
        );
        assert!(email_body(&cache, "missing").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(not(feature = "widget-email"))]
fn run_agent_email(command: EmailCommand) -> Result<()> {
    let _ = command;
    anyhow::bail!("agent email requires building with the widget-email feature")
}

/// True when `~/.config/docket/config.toml` exists. Path-resolution failures
/// (no home dir, etc.) report `true` so an unusual environment doesn't
/// block launch behind a first-run config-seed step.
fn looks_initialized() -> bool {
    config::config_path().map(|p| p.exists()).unwrap_or(true)
}

/// Resolve a `--clear-cache <target>` argument and apply it to the on-disk
/// cache. `target` is `"*"` (the sentinel for "no value passed"), a widget
/// kind (`news`), or `kind@instance` (`news@home`). When `confirm` is true,
/// the user must answer `y` at a [y/N] prompt; otherwise no prompt is shown.
/// A declined prompt is non-fatal — startup continues with the cache intact.
fn run_clear_cache(target: &str, confirm: bool) -> Result<()> {
    let cache = cache::Cache::open_default().context("failed to open cache directory")?;
    let action = ClearAction::parse(target);

    if confirm && !prompt_yes_no(&action.confirm_message())? {
        println!("Cache clear cancelled — continuing without changes.");
        return Ok(());
    }

    match &action {
        ClearAction::All => cache.clear_all()?,
        ClearAction::Kind(kind) => cache.clear_widget(kind)?,
        ClearAction::Instance { kind, instance } => cache.clear_instance(kind, instance)?,
    }
    println!("{}", action.success_message(cache.root()));
    Ok(())
}

enum ClearAction {
    All,
    Kind(String),
    Instance { kind: String, instance: String },
}

impl ClearAction {
    fn parse(target: &str) -> Self {
        if target == "*" || target.is_empty() {
            return Self::All;
        }
        let (kind, instance) = parse_widget_ref(target);
        if instance == "main" && !target.contains('@') {
            Self::Kind(kind)
        } else {
            Self::Instance { kind, instance }
        }
    }

    fn confirm_message(&self) -> String {
        match self {
            Self::All => "Clear ALL cached docket data?".to_string(),
            Self::Kind(kind) => format!("Clear all cached data for widget {kind:?}?"),
            Self::Instance { kind, instance } => {
                format!("Clear cached data for {kind}@{instance}?")
            }
        }
    }

    fn success_message(&self, root: &std::path::Path) -> String {
        match self {
            Self::All => format!("Cleared cache at {}", root.display()),
            Self::Kind(kind) => format!("Cleared cache for widget {kind:?}"),
            Self::Instance { kind, instance } => {
                format!("Cleared cache for {kind}@{instance}")
            }
        }
    }
}

/// Single-line `[y/N]` prompt on stdin. Empty / N / n / EOF all return false.
fn prompt_yes_no(question: &str) -> Result<bool> {
    print!("{question} [y/N]: ");
    io::stdout().flush().ok();
    let stdin = io::stdin();
    let mut buf = String::new();
    let n = stdin
        .lock()
        .read_line(&mut buf)
        .context("failed to read confirmation from stdin")?;
    if n == 0 {
        return Ok(false);
    }
    Ok(matches!(
        buf.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

    // Route logs to ~/.config/docket/docket.log — alt-screen mode would corrupt
    // the dashboard the moment a widget logged to stderr. `tail -f` it when
    // debugging.
    let Ok(dir) = config::config_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("docket.log");
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_accepts_each_cache_clear_form() {
        let all = Cli::try_parse_from(["docket", "--clear-cache"]).unwrap();
        assert_eq!(all.clear_cache.as_deref(), Some("*"));

        let kind = Cli::try_parse_from(["docket", "--clear-cache", "email"]).unwrap();
        assert_eq!(kind.clear_cache.as_deref(), Some("email"));

        let instance =
            Cli::try_parse_from(["docket", "--clear-cache-forced", "feeds@work"]).unwrap();
        assert_eq!(instance.clear_cache_forced.as_deref(), Some("feeds@work"));
    }

    #[test]
    fn cli_accepts_config_and_init_together() {
        let cli =
            Cli::try_parse_from(["docket", "--init", "--config", "/tmp/docket.toml"]).unwrap();
        assert!(cli.init);
        assert_eq!(
            cli.config.as_deref(),
            Some(std::path::Path::new("/tmp/docket.toml"))
        );
    }

    #[test]
    fn cache_clear_targets_resolve_to_the_right_scope() {
        assert!(matches!(ClearAction::parse("*"), ClearAction::All));
        assert!(matches!(ClearAction::parse(""), ClearAction::All));
        assert!(matches!(ClearAction::parse("email"), ClearAction::Kind(k) if k == "email"));
        assert!(
            matches!(ClearAction::parse("feeds@work"), ClearAction::Instance { kind, instance } if kind == "feeds" && instance == "work")
        );
        assert_eq!(
            ClearAction::All.confirm_message(),
            "Clear ALL cached docket data?"
        );
        assert_eq!(
            ClearAction::Kind("email".into()).confirm_message(),
            "Clear all cached data for widget \"email\"?"
        );
        assert_eq!(
            ClearAction::Instance {
                kind: "feeds".into(),
                instance: "work".into()
            }
            .confirm_message(),
            "Clear cached data for feeds@work?"
        );
    }
}

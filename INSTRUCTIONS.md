# Docket setup instructions

Docket has no interactive setup wizard, and no OAuth — `docket --init`
seeds default config files directly to disk, and you configure
everything by hand-editing TOML and filling in app passwords / API
keys. This file walks through the one-time provider setup (CalDAV,
ICS, IMAP) and the moving parts a config file alone can't walk you
through (where to generate an app-specific password, third-party
portals).

If you're stuck, see [Troubleshooting](#troubleshooting) at the bottom or open an issue at https://github.com/nicococo/docket/issues.

---

## CalDAV (iCloud / Fastmail / Nextcloud)

CalDAV is the open standard for calendar sync, authenticated with an
app-specific password. Docket already ships the credentials template —
you just fill it in.

### Apple iCloud

1. Go to https://appleid.apple.com and sign in.
2. *Sign-In and Security* → *App-Specific Passwords* → *+ Generate*.
3. Name it `docket` and copy the generated 4-block password (looks like `abcd-efgh-ijkl-mnop`).
4. Edit `~/.config/docket/credentials/caldav.toml`:

   ```toml
   server = "https://caldav.icloud.com"
   username = "your.apple.id@icloud.com"
   app_password = "abcd-efgh-ijkl-mnop"
   ```
5. Add a `[[providers]]` block with `kind = "caldav"` to `~/.config/docket/calendar.toml`:

   ```toml
   [[providers]]
   kind = "caldav"
   ```

   `credentials/caldav.toml` currently holds one server/username/password —
   CalDAV is single-account. For multiple calendar accounts, use ICS
   (below), which supports any number of labeled feeds.

### Fastmail

1. https://www.fastmail.com/settings/security/devicekeys → *New app password* → scope: *CalDAV*.
2. Same `caldav.toml` layout, with `server = "https://caldav.fastmail.com"`.

### Nextcloud, Synology, generic CalDAV

Use your normal username + an app-specific password from the server's UI. Server URL is whatever the server exposes (e.g. `https://nextcloud.example.com/remote.php/dav`).

---

## ICS / webcal feeds (multiple calendars, incl. Google Calendar)

A plain ICS feed is a single HTTP GET against a static `.ics` URL — no
CalDAV discovery, no OAuth, no per-app registration. It's read-only and
typically only refreshes every few hours server-side, so it's not
real-time, but it's the simplest way to pull in a calendar — including
**Google Calendar**, via its "secret address in iCal format".

Unlike CalDAV, ICS supports any number of **labeled feeds** in one
credentials file, so it's the way to add multiple calendar accounts.

### Get a feed URL

- **Google Calendar**: Settings → pick a calendar under "Settings for
  my calendars" → *Integrate calendar* → copy "Secret address in iCal
  format".
- Any other calendar app that publishes a public or secret iCalendar
  export works the same way.

### Setup

Edit `~/.config/docket/credentials/ics.toml`, adding one `[[feeds]]`
block per calendar:

```toml
[[feeds]]
label = "work"
url = "https://calendar.google.com/calendar/ical/x/basic.ics"

[[feeds]]
label = "family"
url = "https://calendar.google.com/calendar/ical/y/basic.ics"
```

Then add one `[[providers]]` block per feed in `~/.config/docket/calendar.toml`,
matching `account` to the feed's `label`:

```toml
[[providers]]
kind = "ics"
account = "work"

[[providers]]
kind = "ics"
account = "family"
```

Each account's `calendar_colors` key follows `kind/label` for a named
account (`"ics/work:primary"`), or just `kind` for the account labeled
`"default"` (`"ics:primary"`) — so different feeds never share a color.

---

## IMAP (Gmail / iCloud / Fastmail / self-hosted)

docket's only email backend. You provide host, port, username, and an app-specific password and docket connects directly. Works against any IMAP4rev1 server.

### Per-provider hosts + app-password recipes

| Provider | Host | Port | App-password URL |
|---|---|---|---|
| Gmail | `imap.gmail.com` | 993 | https://myaccount.google.com/ → Security → 2-Step Verification → App passwords |
| iCloud | `imap.mail.me.com` | 993 | https://appleid.apple.com → Sign-In and Security → App-Specific Passwords |
| Fastmail | `imap.fastmail.com` | 993 | https://www.fastmail.com/settings/security/devicekeys → *New app password* → scope *IMAP* |
| Yahoo | `imap.mail.yahoo.com` | 993 | https://login.yahoo.com/account/security → Generate app password |
| Outlook / O365 | `outlook.office365.com` | 993 | ⚠️ Microsoft has been phasing out IMAP basic auth for these accounts — may not work; no OAuth alternative in docket |
| Self-hosted | whatever your server exposes | usually 993 | depends on the server (Mailcow / Dovecot / etc.) |

(Gmail requires 2-Step Verification to be enabled before you can generate app passwords. iCloud and Fastmail also force app passwords for third-party clients — your account password won't work.)

### Setup

Drop a file at `~/.config/docket/credentials/imap.toml`:

```toml
host = "imap.gmail.com"
port = 993
use_tls = true
username = "alice@gmail.com"
app_password = "abcd-efgh-ijkl-mnop"
```

Then in `email.toml`:

```toml
provider = "imap"
folders = ["INBOX"]
```

Docket will connect lazily on the first fetch.

---

## LLM provider key (optional, for summaries)

The news + email widgets can summarise expanded items using a
configurable LLM. Docket ships two providers out of the box:
**Anthropic (Claude)** and **OpenAI (GPT)**. You pick one — the
widgets call whichever is active in `llm.toml`.

### Anthropic (Claude)

1. https://console.anthropic.com/ → *Get API Keys* → create a key.
2. Edit `~/.config/docket/credentials/anthropic_key.toml`:

   ```toml
   api_key = "sk-ant-..."
   ```

### OpenAI (GPT)

1. https://platform.openai.com/api-keys → *Create new secret key*.
2. Edit `~/.config/docket/credentials/openai_key.toml`:

   ```toml
   api_key = "sk-..."
   ```
3. The default OpenAI model is `gpt-5-mini`. Change it in `llm.toml`
   if you want a different model — the field is sent verbatim to the
   OpenAI Chat Completions API, so any model name your account can
   call (e.g. `gpt-4o-mini`, `gpt-4o`) works.

### Activating LLM features

After the key is on disk:

- `llm.toml` carries `[provider] name = "anthropic"` or `"openai"` —
  set this by hand to pick which provider is active.
- `summarize_with_llm = true` in `news.toml` / `email.toml` opts each
  widget into summaries. Both default to `true` once a key is configured.

If no key is configured (or `enabled = false` in `llm.toml`), the
`s summarize` keyboard hint stays hidden in the email widget; the
news widget renders the raw RSS excerpt instead.

---

## Troubleshooting

### "Fill in the template at … then retry" but I edited the file and it still complains

The credentials check re-reads the file on every connection attempt.
Double-check:

- File path is exactly `~/.config/docket/credentials/<name>.toml`
  (`caldav.toml`, `ics.toml`, `imap.toml`).
- Values are quoted TOML strings: `username = "alice@example.com"`.
- Neither value still starts with `REPLACE_WITH_…`.

### Calendar / email shows "Last fetch failed: …"

Read the message — it carries the provider's error verbatim. Common causes:

- **Wrong password**: most providers require an app-specific password,
  not your account password — see the per-provider sections above.
- **Network**: corporate proxies sometimes interact badly with
  CalDAV/IMAP. Try from a non-corporate network or set `HTTPS_PROXY`
  if needed.

### I want to start completely fresh

```bash
rm -rf ~/.config/docket
docket --init
```

This wipes everything — config, tokens, cache. `docket --init` seeds fresh defaults from docket's built-in templates.

---

## What lives where on disk

```
~/.config/docket/
├── config.toml               # [global] + [layout] + [[layout.cells]]
├── colorschemes.toml         # named [schemes.*] palettes
├── calendar.toml  news.toml  feeds@<instance>.toml
├── resources.toml  email.toml
├── notes.toml  llm.toml
├── credentials/               # account secrets (0700)
│   ├── caldav.toml  ics.toml  imap.toml
│   ├── anthropic_key.toml  openai_key.toml
├── notes/<instance>/<id>.md   # each note as a plain markdown file
└── .runtime_state.toml  docket.log
```

Every `.toml` is plain text — edit in your favourite editor and either restart docket or hit `:reload` from the runtime command bar.

---

## Further reading

- `README.md` — install, keybindings, color schemes, configuration.
- `AGENTS.md` — architecture overview for contributors and AI assistants.
- https://github.com/nicococo/docket — source, issues, releases.

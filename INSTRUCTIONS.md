# Connect your sources

Start docket once to create `~/.config/docket/`. Press `s` (`Ctrl+G` also works) for settings. `Tab` switches pages, arrows select, `Enter` edits, `Ctrl+U` clears a field, `a` adds a source/account, `x` removes the last one, and `Ctrl+S` saves. Field descriptions explain formats and where to find setup details. Credentials live in `~/.config/docket/credentials/`; keep them private and use app passwords where available.

## Calendar

The default calendar is local (`calendar.ics`). For CalDAV, edit `credentials/caldav.toml`:

```toml
server = "https://caldav.icloud.com"
username = "you@example.com"
app_password = "your-app-password"
```

Add to `config.toml`:

```toml
[[calendar.providers]]
kind = "caldav"
```

For an ICS feed, add it to `credentials/ics.toml` and reference its label:

```toml
# credentials/ics.toml
[[feeds]]
label = "work"
url = "https://example.com/calendar.ics"
```

```toml
# config.toml
[[calendar.providers]]
kind = "ics"
account = "work"
```

Repeat both blocks for additional ICS calendars. Treat private feed URLs as secrets.

## Email

Add an account in Email settings, then fill its server, username, and app password fields. For manual setup, create `credentials/imap.toml`:

```toml
host = "imap.gmail.com"
port = 993
use_tls = true
username = "you@example.com"
app_password = "your-app-password"
```

Set `provider = "imap"` under `[email]` in `config.toml`. For multiple accounts, use `[[email.accounts]]` entries with `label` and `folders`, and name the matching credential files `imap_<label>.toml`. App-password setup varies by provider; some services do not support IMAP password authentication.

## Feeds and notes

Each `[[feeds.feeds]]` entry in `config.toml` has a `topic` and `url`. Notes are Markdown; set `[notes] notes_dir` to use a different folder.

Docket reloads configuration changes automatically. If a connection fails, check the error in the pane, the credentials and `~/.config/docket/docket.log`.

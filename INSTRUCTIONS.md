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

## Agent and CLI setup

Named sources can be managed without opening the TUI:

```sh
docket setup feed add work https://example.com/rss.xml
docket setup feed remove work
python3 -c 'import getpass, sys; sys.stdout.write(getpass.getpass("App password: "))' | docket setup email add work --host imap.example.com --username you@example.com
docket setup email remove work
docket setup calendar add personal --kind local
python3 -c 'import getpass, sys; sys.stdout.write(getpass.getpass("Secret ICS URL: ").strip())' | docket setup calendar add work --kind ics
docket setup calendar add work --kind caldav --server https://caldav.example.com --username you@example.com
docket setup calendar remove work
```

Email and CalDAV app passwords and private ICS URLs are read from stdin and saved under `credentials/` with private file permissions. Add `--calendar-id URL` (repeatable) to restrict a CalDAV source. Names contain only letters, digits, `_` or `-`; remove commands require a unique exact name. Use `docket --config PATH setup …` to target another config directory.

## Agent-guided Gmail and Google Calendar setup

Agents: when asked to connect email and calendars, guide the user through the
following steps, one account at a time. Ask for missing details and help resolve
errors before continuing. Never request passwords or secret calendar URLs in chat.

1. Find the active `docket` executable (`command -v docket`, or the active plugin
   root in Herdr's `~/.config/herdr/plugins.json`). Check `docket setup --help` and
   its email/calendar subcommands. If this build lacks setup commands, explain
   that it needs a newer build; do not invent commands. Inspect configured source
   names without printing credentials, and preserve existing sources.
2. Ask for each email address and desired account name, such as `personal` or
   `work`. Use that name for both email and calendar; names allow letters, digits,
   `_` and `-`. Confirm whether the user wants the primary calendar or additional
   calendars (each ICS feed needs a unique calendar source name).
3. For Gmail, use `imap.gmail.com`, port `993`, TLS, the full email address as the
   username, and `INBOX`. Have the user sign into the matching Google account,
   enable 2-Step Verification if needed, and create an app password named for
   docket at <https://myaccount.google.com/apppasswords>. Use the app password,
   not the regular Google password. If the option is missing, consult
   [Google's app-password help](https://support.google.com/accounts/answer/185833).
4. Give the user this command with the verified executable path, account name
   and email address substituted. Run it in their own interactive terminal so
   they can enter the app password at a hidden prompt. Never place secrets in
   command arguments, environment variables, scripts, chat, or shell history.

   ```sh
   python3 -c 'import getpass, sys; sys.stdout.write(getpass.getpass("Google app password: ").replace(" ", ""))' |
     docket setup email add personal --host imap.gmail.com --username you@gmail.com
   ```

5. For the calendar, have the user open
   <https://calendar.google.com/calendar/r/settings> in the matching Google
   account. Under **Settings for my calendars**, select the calendar, then
   **Integrate calendar → Secret address in iCal format**. This URL grants
   read-only access; no app password is needed. Keep it private and do not make
   the calendar public. If it is missing, consult
   [Google's calendar help](https://support.google.com/calendar/answer/37648).
   Have the user paste it at the hidden prompt in this command, again substituting
   the executable and account name:

   ```sh
   python3 -c 'import getpass, sys; sys.stdout.write(getpass.getpass("Secret ICS URL: ").strip())' |
     docket setup calendar add personal --kind ics
   ```

6. Repeat for each account using its own app password and calendar URL. Ask for
   success messages or redacted errors only. `Added …` confirms configuration was
   saved; open docket and verify that messages and events actually load before
   claiming the connections work. ICS feeds are read-only and updates may lag.

If email setup reports `unsupported rust type`, check for an older setup CLI
serializing standalone strings with `toml_edit::ser::to_string`. TOML string
values should use `toml_edit::Value::from(text.as_str()).to_string()` instead.
Check for partial writes before retrying; use a corrected build tested with dummy
credentials in a temporary configuration. Never reuse a temporary executable path
from someone else's session or expose real credentials while debugging.

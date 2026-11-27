# Docket quick guide

Docket is a fixed four-pane terminal cockpit for the working moment: scan unread and recent email, glance at calendars and feeds, and capture next steps. It is for quick catch-up, not long-term email management. People and coding agents share the same notes; Herdr opens and closes the dashboard with a shortcut.

## Connect sources

Start docket and press `s` for settings. `Tab` switches settings pages; arrows select fields; `Enter` edits; `a` adds a source/account; `x` removes one; `Ctrl+S` saves. Credentials belong in docket’s private `credentials/` directory. Use provider app passwords, never primary passwords.

- **Email:** Gmail uses IMAP at `imap.gmail.com:993` with TLS and a Google app password. Other IMAP providers need their server, username, and app password.
- **Google Calendar:** add the calendar’s private **Secret address in iCal format** as an ICS source. It is read-only. Keep the URL private. [Google instructions](https://support.google.com/calendar/answer/37648).
- **CalDAV:** use the server URL, account name, and an app password. Local `calendar.ics` is available by default.
- **Feeds:** add RSS or Atom URLs in Feeds settings.

## Set up with an agent

Use this prompt:

> Read https://github.com/nicococo/docket/blob/main/INSTRUCTIONS.md. Help me set up Gmail and Google Calendar in docket. Find and verify the installed docket CLI first. Check existing sources and preserve them. Ask for needed details one at a time. Never ask me to share passwords or private calendar URLs in chat; have me enter each secret in a hidden local prompt. Configure one account at a time, then help me verify that email and calendar data load.

Agent procedure:

1. Find the CLI. Herdr’s managed plugin keeps the executable under its plugin root rather than on `PATH`; resolve it from Herdr’s registry if no standalone binary is installed:

   ```sh
   docket=$(command -v docket || true)
   if [ -z "$docket" ]; then
   plugin_root=$(python3 -c 'import json,os,pathlib; p=pathlib.Path(os.environ.get("XDG_CONFIG_HOME",pathlib.Path.home()/".config"))/"herdr/plugins.json"; d=json.loads(p.read_text()); print(next(x["plugin_root"] for x in d if x["plugin_id"]=="nicococo.docket" and x["enabled"]))')
   docket="$plugin_root/target/release/docket"
   fi
   "$docket" setup --help
   ```

   Use `"$docket"` for subsequent commands. Do not build a second copy if the installed binary exists.
2. Ask for an account label and email address. Use the same label for that account’s calendar, unless the user prefers otherwise. Check configured names without displaying credentials.
3. For Gmail, confirm 2-Step Verification and direct the user to create an app password at <https://myaccount.google.com/apppasswords>. In the user’s terminal, use:

   ```sh
   python3 -c 'import getpass,sys; sys.stdout.write(getpass.getpass("Google app password: ").replace(" ", ""))' | "$docket" setup email add personal --host imap.gmail.com --username you@gmail.com
   ```

4. Have the user copy **Secret address in iCal format** from Google Calendar settings. Enter it locally, hidden:

   ```sh
   python3 -c 'import getpass,sys; sys.stdout.write(getpass.getpass("Secret ICS URL: ").strip())' | "$docket" setup calendar add personal --kind ics
   ```

   Replace `personal` and `you@gmail.com` with the confirmed values. Never put secrets in chat, command arguments, environment variables, scripts, or shell history.
5. Check the `Added …` responses, then open docket and verify both sources load. If setup fails, request only the redacted error. Do not claim a connection works from a successful config write alone.

For other sources, use `docket setup feed add|remove`, `docket setup email add|remove`, and `docket setup calendar add|remove`. Use `--help` for exact options. Email and CalDAV app passwords and ICS URLs are read from stdin. See the relevant provider’s own help page for server details.

## Notes and agent memory

Notes are plain Markdown under the notes folder configured in Settings (`~/.config/docket/notes/` by default). They work in Obsidian. Docket interprets `>` in a note’s first line as a path, and renders board notes from Markdown headings and task lists. Point `notes_dir` at a folder in a private GitHub repository or Obsidian vault to sync it with Git; Git handles syncing and conflicts, not docket. Avoid editing the same note on two machines at once. Keep credentials out of notes.

Agents can use `docket agent notes list|read|add` for shared notes. Use scoped memory for agent knowledge:

```sh
docket agent memory id --role implementer --agent omp
printf 'Release workflow\n\nUse the locked release build.\n' | docket agent memory add --role implementer --agent omp
docket agent memory list --role implementer --agent omp
docket agent memory read 'Agents-docket-implementer-omp-Release workflow'
```

The identity is project/role/agent. Project defaults to the current Git root (or current directory outside Git); `DOCKET_PROJECT`, `DOCKET_AGENT_ROLE`, and `DOCKET_AGENT_ID` or `--project`, `--role`, `--agent` select stable scopes. Role and agent default to `general/shared`, so use explicit stable labels when separating agents. `id` prints the identity and path. `add` and `edit <id>` read Markdown from stdin; `delete <id>` removes only a note in the selected scope. IDs come from `list` output.

Memory titles are stored as `Agents>project>role>agent>title`, using docket’s path grouping. That keeps agent notes apart from human notes and from other projects, roles, and agents while remaining ordinary `.md` files. Docket lists and filters the Markdown files directly; no separate database is needed. Before work, read relevant memories; save durable decisions, constraints, discoveries, and handoff state when useful; edit or delete stale notes. Keep secrets and transient speculation out of memory.

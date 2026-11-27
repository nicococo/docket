---
name: docket
description: Use Docket to configure its sources, read cached dashboard data, and share Markdown notes or scoped agent memory. Use when helping with Docket setup or when working with information and notes stored in Docket.
---

# Docket

Docket is a terminal dashboard and shared Markdown store. Use this skill when configuring Docket or reading and maintaining its notes, agent memory, and cached data.

## Find the CLI

Prefer an installed executable and avoid building a duplicate:

```sh
docket=$(command -v docket || true)
```

When running inside an installed Herdr plugin, check its root:

```sh
if [ -z "$docket" ] && [ -n "$HERDR_PLUGIN_ROOT" ]; then
  docket="$HERDR_PLUGIN_ROOT/target/release/docket"
fi
```

Otherwise, on Linux or macOS, check Herdr's plugin registry:

```sh
if [ -z "$docket" ]; then
  plugin_root=$(python3 -c 'import json,os,pathlib; p=pathlib.Path(os.environ.get("XDG_CONFIG_HOME",pathlib.Path.home()/".config")) / "herdr/plugins.json"; d=json.loads(p.read_text()); print(next(x["plugin_root"] for x in d if x["plugin_id"]=="nicococo.docket" and x["enabled"]))' 2>/dev/null || true)
  if [ -n "$plugin_root" ]; then docket="$plugin_root/target/release/docket"; fi
fi
```

If no executable is found, explain how to install Docket from the [README](https://github.com/nicococo/docket#install). Before using commands, check `"$docket" agent --help` or the relevant subcommand's `--help`.

## Configure sources

Read the [Docket setup guide](https://github.com/nicococo/docket/blob/main/INSTRUCTIONS.md) before changing configuration. Ask for missing details one at a time, preserve existing sources, and configure one source at a time. Use `docket setup feed`, `docket setup email`, and `docket setup calendar`; check each command's help for options.

Never ask the user to paste passwords, app passwords, or private calendar URLs into chat. Have them enter secrets through a hidden local prompt piped to Docket's stdin, following the setup guide. Do not put secrets in arguments, environment variables, shell history, notes, or memory. A successful config write is not proof the source works: verify the data loads in Docket before reporting setup complete.

## Read and write notes

Notes are ordinary Markdown files shared by Docket, Obsidian, people, and agents. Use:

```sh
"$docket" agent notes list
"$docket" agent notes read <id>
printf 'Title\n\nNote body.\n' | "$docket" agent notes add
```

Use shared notes for information useful to people and multiple agents. Titles can use Docket's `>` path convention to group notes. Keep secrets and sensitive personal data out of notes.

## Maintain scoped agent memory

Use scoped memory for durable project context, decisions, constraints, discoveries, and handoffs. First inspect the resolved namespace and existing notes:

```sh
"$docket" agent memory id --role <role> --agent <stable-agent-label>
"$docket" agent memory list --role <role> --agent <stable-agent-label>
```

Read relevant memory before work. Add or edit concise, durable information when it will help future work; avoid transient status and speculation. Use stable role and agent labels so separate agents do not write into the same namespace by accident. The project defaults to the current Git root; `DOCKET_PROJECT`, `DOCKET_AGENT_ROLE`, and `DOCKET_AGENT_ID` can set stable defaults.

```sh
printf 'Decision: keep the importer idempotent\n\nReason and constraints...\n' |
  "$docket" agent memory add --role implementer --agent <stable-agent-label>
"$docket" agent memory read <id> --role <role> --agent <stable-agent-label>
```

Use IDs from the selected scope's `list` output for `read`, `edit`, and `delete`. Confirm the scope with `id` before editing or deleting. Docket stores memory as Markdown under the notes directory; no separate database is involved. If notes are Git-synced, avoid simultaneous edits to the same file and resolve conflicts before continuing.

## Read cached dashboard data

The agent CLI can inspect cached data without changing the source:

```sh
"$docket" agent calendar list
"$docket" agent feeds list
"$docket" agent email list
"$docket" agent email read <id>
```

These commands read Docket's local cache; they do not guarantee a live refresh. Email read prints a cached message body without changing its read state. Be careful with private email and calendar data: return only what is needed for the user's request.

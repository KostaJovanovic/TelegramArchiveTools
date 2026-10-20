# Driving TelegramTools from an AI agent

This is for a second project in which a model summarises a Telegram chat and
maps it out, and needs the chat's database brought up to date first. The order
is: **sync** the databases (network), then **analyse** them (offline), then let
the model read what the analyser wrote. Everything below is plain commands with
exit codes, so any agent that can run a shell can do it.

## Once, by a person

The agent cannot do these. They involve a login code and a password.

1. **Build or copy the exe.** `save.bat build` writes `dist\TelegramTools.exe`.
   Its state (`TelegramToolsData\`) and its exports (`Exports\`) live beside
   it, so choose its folder once and leave it there.
2. **Sign in:** `TelegramTools login` in PowerShell (from cmd.exe:
   `start /wait TelegramTools login`). It asks for the phone number, the code
   Telegram sends, and the two-factor password if there is one. The session is
   saved, so this is needed once, not on every run.
3. **Make the first database for each chat the agent should follow:**

   ```powershell
   TelegramTools chats                   # find the chat and its id
   TelegramTools export --db 1234567890  # the id, or the title
   ```

   This reads the whole chat once and writes
   `Exports\<chat>\<chat>.sqlite`. Every later `sync` updates exactly the
   databases that exist in `Exports\`, so this step decides what the agent
   covers. The window's Database mode makes the same files.

## Every time, by the agent

### 1. Update the databases

```powershell
& "C:\path\to\TelegramTools.exe" sync "D:\wherever\the\databases\are" --json
```

- Syncs **every** database in that folder, one chat after another, on one
  connection, writing each update back into the same file. Without a folder it
  uses the destination set in the window (`Exports\` beside the exe by
  default). `--only <id or title>` (repeatable) narrows it.
- **Which databases it finds:** `<folder>\<name>\<name>.sqlite` (the folder
  and the file share a name, which is how this tool writes them) and
  `<folder>\<name>.sqlite`. Nothing deeper. The name doesn't need to match
  the chat's current title: the chat is identified by what is inside the file,
  so a chat renamed on Telegram still syncs into its old database.
- A file holding several chats (from a very old build) is listed under
  `unreadable` and left alone. `TelegramTools split <file>` breaks it up.
- One chat in several files — a group renamed between syncs, before the
  window looked databases up by what is inside them — syncs into the one
  that last finished a sync. `TelegramTools merge [<folder>] [--dry-run]`
  folds the others into it and moves their folders into its `merged\`.
- A sync reads what is new plus the newest 500 messages again, to pick up
  edits, reactions and deletions. **It takes seconds to minutes, not hours.**
  `--full` re-reads the whole history. That is slow on a large chat and is
  only needed for deletions older than that window. Don't pass it unless asked.
- **stdout is exactly one JSON document**; all narration goes to stderr. Parse
  stdout, and keep stderr for the log.
- Allow a long timeout (30 minutes or more). Telegram rate-limits, and the
  tool waits rather than failing (`rate limited, waiting Ns` on stderr).

The JSON:

```json
{
  "folder": "C:\\Tools\\Exports",
  "full": false,
  "synced": 2,
  "not_synced": 0,
  "chats": [
    {
      "chat_id": 1234567890,
      "title": "Dev Team",
      "database": "C:\\Tools\\Exports\\Dev Team\\Dev Team.sqlite",
      "status": "synced",
      "new": 41, "changed": 3, "deleted": 0,
      "files_stored": 5, "media_missing": 0,
      "messages_stored": 18234, "messages_read": 541
    }
  ],
  "unreadable": []
}
```

`status` is one of:

| status | meaning | what to do |
|---|---|---|
| `synced` | read to the end; the database is current | carry on |
| `incomplete` | the read stopped early (network, Telegram errors) | carry on with what is there, say so; retry later |
| `failed` | the chat could not be synced at all; `error` says why | carry on with the old data, say so |
| `cancelled` | Ctrl-C reached the process before this chat | as `failed` |
| `not_in_dialogs` | the account no longer sees this chat (left, removed, migrated) | the database still holds everything up to then |

**Exit code:** 0 when every chat is `synced` or `not_in_dialogs`; 1 otherwise,
or on an error before any chat started. The second kind prints no JSON, only
`error: …` on stderr. The two errors worth recognising there:

- `not signed in — run TelegramTools login first`: stop and ask the person
  to sign in. Don't attempt it.
- `the TelegramTools window is open on this data folder`: the window and a
  sync would both write the same databases, so the sync refuses. Ask the person
  to close it.

A sync only adds to a database: messages are never removed, only marked when
Telegram deleted them. So analysing after a failed sync is safe. The data
just isn't current.

### 2. Analyse

```powershell
& "C:\path\to\TelegramTools.exe" analyse "D:\wherever\the\databases\are" --refresh --digest
```

The same folder as the sync. Offline. For each database whose report is older
than the database, it writes into that database's own folder:

- `report.html`: the full report, self-contained, for a person.
- `analysis\digest.jsonl`: **the file for the model.** The whole chat, one
  message per line, oldest first: `id`, `t` (local time, to the minute),
  `topic`, `who`, `text`, `re` (the id it replies to), `media`, `reactions`, a
  `service` action name, and for a document whose text could be read, `doc`
  with its own `date` and how that date was found.
- `analysis\EVENTS.md`: instructions written *for a model*. They cover how to
  read the digest, how to date documents, and the shape of `events.json`.
  Give it to the model along with the digest.

Add `--force` to redo every report whether it changed or not. To analyse one
chat only, name its database:
`TelegramTools analyse "Exports\Dev Team\Dev Team.sqlite" --digest`.

### 3. (Optional) Put the model's map into the report

If the model writes an `events.json` following `EVENTS.md` (dated events, each
citing the message ids it rests on) into `Exports\<chat>\` or
`Exports\<chat>\analysis\`, the next `analyse --refresh` pins those events
to the report's timeline. A new or changed `events.json` counts as a change, so
`--refresh` picks it up without `--force`.

### 4. (Optional) Have the local model summarise a range

```powershell
& "C:\path\to\TelegramTools.exe" summarise "<databases folder>\<chat>\<chat>.sqlite" --from 2026-01-05 --to 2026-01-11 --detailed
```

Offline but for `127.0.0.1`: it needs a llama-server already running on this
machine (port 8080, or `--port`) with a Gemma loaded, and refuses another
model unless given `--any-model`. **stdout is the summary**, streamed as it is
written; stderr narrates counts and timings. The summary is also saved under
`<chat folder>\analysis\summaries\`. `--short`, `--detailed`, `--sections`,
`--by-topic`, `--topic <name>` and `--lang sr|en|chat` choose its shape;
`TelegramTools summarise --help` lists them. The window's **Summaries** view
does the same, and can start the server itself; it also reads the text in the
range's photos first. The command line uses those readings where the window
has made them, and reads no photos itself.

## Other read-only commands an agent may use

None of these writes anything, and all but `chats` are offline:

```powershell
TelegramTools chats --json             # (online) every chat: id, title, kind, and its database if any
TelegramTools search "word" --limit 20 # every message containing it, across the databases
TelegramTools doctor                   # what is wrong with the export folder
TelegramTools analyse --help
```

The databases are SQLite and may also be read directly, **read-only**. Open
them with `?mode=ro` (or `file:…?mode=ro` URI), never write to them, and don't
hold one open during a sync. `DATABASE.md` documents the schema. The digest is
almost always the easier input.

## Rules for the agent

- **Never run `login` or `logout`, and never read `TelegramToolsData\`.** It
  holds a Telegram session key that is as good as the password.
- Don't pass `--full` unless the person asked for it.
- The exports and the digest are other people's conversations. Keep them in
  the project that needs them, and don't paste them into logs, issues or
  anywhere outside it.
- From `cmd.exe` the exe returns at once unless run with `start /wait`.
  PowerShell, bash and any program capturing its output wait normally.

## A paste-in block for the other project's instructions

```markdown
## Updating the Telegram data

Before summarising or mapping a chat, bring its database up to date:

1. Run `& "C:\path\to\TelegramTools.exe" sync "<databases folder>" --json` (timeout: 30 min).
   Parse stdout as JSON. For each chat, `status` is `synced`, `incomplete`,
   `failed`, `cancelled` or `not_in_dialogs`. Anything but `synced`: carry on
   with the stored data and tell the user which chat is not current and why.
   If it exits 1 with no JSON and stderr says "not signed in" or "window is
   open", stop and ask the user. Never run `login`, never pass `--full` unless
   asked, never read `TelegramToolsData\`.
2. Run `& "C:\path\to\TelegramTools.exe" analyse "<databases folder>" --refresh --digest`.
3. Read `<databases folder>\<chat>\analysis\EVENTS.md` (the instructions) and
   `<databases folder>\<chat>\analysis\digest.jsonl` (the chat, one message per line).
4. To put the map on the report's timeline, write `events.json` as `EVENTS.md`
   describes into `<databases folder>\<chat>\analysis\` and run step 2 again.
```

# Summaries: what happened in a chat, by the local Gemma

The plan for a new rail view that asks a local model to summarise a date range
of one chat. The phases at the end say what comes first.

**Status (2026-10-14): all four phases built.** `crates/tga-sum` (pipeline,
loopback client, server keeper), `tga_run::summary` (reading, the CLI, the
window's workers), `tt-app/src/shell/summaries.rs` (the view) and Settings ›
Summaries. Tested against a stand-in server
(`crates/tga-sum/tests/fake_server.rs`) and photographed with the synthetic
export; **not yet run against Gemma**, and the prompts are untuned until the
account holder has judged real output.

What changed from the plan below while building it:

- **A summary is saved as soon as it is written**, as the CLI does, rather than
  on a Save button; the **Saved** list and "a range already summarised opens at
  once" rest on that. Running again with the same options overwrites the file.
- **The command line attaches only.** Starting a server needs the paths in
  Settings, which `tga-run` cannot reach; the window starts one.
- **The quick ranges** are Last day / Last 7 days / Last 30 days, counted back
  from the chat's last message, not from today: a chat may have ended.
- **The estimate is counted from text length** (three characters a token),
  without the documents; the cuts themselves are counted by the server's
  tokenizer.
- **ReVision's server and models, with no button.** The first build had a
  "Find ReVision's" button that copied the paths into Settings; the account
  holder did not want to find anything. A `Model\` folder beside the exe was
  tried next and dropped: they asked for the same server and the same model
  folder as ReVision. Blank paths now mean ReVision's data folder, found on
  its own (`server::revision_data`), shown in Settings with **Open**.
- **Merging says where it is, and is kept (2026-10-15).** A long range came
  to 1,332 notes, 132,403 tokens, and the status read "merging to fit 14000"
  for over ten minutes. A round is a call per group of notes, about thirty
  here, so the status now names the round, the group and the minutes left,
  timed from the calls so far. Each group's merged notes are cached like a
  part's, keyed by the merge prompt and the group's text, so a question about
  the same days, or a re-run, does not merge them again.
- **Independent of ReVision (2026-10-15, the account holder's ask).** They
  reversed the decision above: the app should not depend on ReVision. Now one
  **models folder** (`summary_models`, default `Models\` beside the exe),
  chosen with Browse. It holds `llama-server.exe` and the `.gguf` files, each
  directly or one folder down, so an unzipped llama.cpp release or a model's
  own folder needs no setting. **Model** is a dropdown of the folder's models
  (projectors, embedding models, the OCR and later pieces of a split model
  left out). The first Gemma is used until one is chosen, or when the chosen
  file has gone. A server already on the port is accepted if it runs the
  chosen file even when it is not a Gemma. The OCR comes from the same folder,
  and in a folder of its own any `mmproj` counts as its projector, since
  downloads often name it `mmproj-F16.gguf`. *How to add a model* sits under
  the section, a short version of `docs/MODELS.md`: llama.cpp from GitHub,
  then a GGUF from Hugging Face chosen by card memory. `revision_data` and the
  "ReVision's llama-server" naming are gone. ReVision's numbers stay: they
  were measured there.
- **Photos are read (2026-10-15, the account holder's choice).** Before Gemma,
  the range's photos go through ReVision's OCR: PaddleOCR-VL 1.5 from
  `models\paddleocr-vl\`, its profile (`-c 32768 -ngl 99 --jinja -np 4`, the
  projector on the card, the model's chat template), four at a time, `OCR:` at
  temperature 0, then ReVision's loop cut and Cyrillic fix, and fewer than 25
  letters counts as no text (`tga_sum::ocr`, after `studio/images.py`). The
  text goes into the line as `[photo with text: …]`, capped at 1,500
  characters; `prompt::VERSION` went to 2 for the sentence explaining it.
  - **One model on the card.** `Keeper::ensure` takes a `Want` and stops the
    server it started when the other model is wanted, so a run is OCR, then
    Gemma (a reload of Gemma only when there were photos never read). A
    foreign server is never stopped: ReVision's Gemma on the port means the
    photos are skipped, and the status line says how many.
  - **Read once.** `cache\photos.json` keeps each reading, empty ones too,
    keyed by the OCR's file name and the picture: `kind:file_id` in a
    database, path and size in a folder export. Saved every 40 photos, so a
    stopped run keeps what it read. The command line uses these readings and
    starts no OCR.
  - **Not like ReVision:** no image decoder is linked, so WebP is skipped and
    no photo is shrunk to 1,920 px; and a capital in a word that is not all
    capitals stays capital in the Cyrillic fix (`Йован` → `Јован`).
  - **Off switch:** Settings › Summaries › Read text in photos
    (`summary_photos`, on by default).
- **ReVision's sizes and rules (2026-10-15).** A summary skipped a discussion
  the account holder knew mattered. Parts were 8,000 tokens and one call took
  up to 16,000 — while ReVision's 47%→79% finding, quoted in the code, was
  measured at 4,500. Now: parts of 4,500; one call only up to one part; at most
  14,000 tokens of notes or days in any other call; messages cut at 1,500
  characters; trivial messages dropped unless answered, all as ReVision. The
  notes prompt took ReVision's `events.md` rules in general wording — a
  discussion is one note from start to outcome, with who held what; one that
  ran long is `major` even with no decision; every meeting is its own note —
  and the final prompt says discussions count. `prompt::VERSION` 3. Not taken:
  ReVision's 3-hour session cut (days and the longest gap still decide), its
  facts and glossary block, and its `PIN`/`obrisano`/`⏎` markings.
- **Loaded while the view is open (2026-10-15).** The idle clock unloaded
  Gemma two minutes after a summary, so the first question after reading one
  waited for the model to load again. Now the clock runs only after the
  Summaries view is left (the Viewer, reached from a citation, counts as
  still here); `Unload after` is minutes after leaving.
- **Ask about the days (2026-10-15).** A question box under the summary.
  `tga_sum::ground` reads the form's days — whole up to 14,000 tokens, else
  the same notes a summary takes, from the cache — and `answer` streams a
  reply with checked citations, with up to ~4,000 tokens of the conversation
  so far. The grounding is kept per chat, days and scope; other days start a
  new conversation. One model call at a time: a question waits for a summary
  and the other way round.
- **Answers that keep things apart (2026-10-15).** Over a long range, Gemma
  answered from notes, put one person on both sides of a dispute between two
  similarly named groups, cited ten ids a claim, and when told it was wrong,
  invented a reconciliation. Four changes:
  - **The messages behind the notes.** Over notes, a question first gets a
    pick call (JSON, at most 12 note numbers, given the last 3 turns so a
    follow-up resolves); the answer then reads the picked notes "for
    orientation" and their messages ±3, ±1 or ±0 — then fewer notes — with each
    shown message's reply parent, fitted beside the conversation by
    `/tokenize`. Only those ids may be cited. Nothing picked → all the notes,
    as before.
  - **The chat prompt's rules:** answer from the input only; keep similarly
    named groups, meetings, documents and proposals apart and say which is
    meant; a person's position only from their own messages, per thing, never
    inferred from whom they sided with, both stances said when they differ,
    changes dated; when told an answer was wrong, re-check and say which version
    the input supports — no invented reconciliation, no apology for what the
    input says; cite the one to three most direct messages. Not taken: the
    model's own suggestion of a forced `[Name]: [stance] on [object]` answer
    format.
  - **The notes prompt** names each position's holders one by one ("X wanted A;
    Y and Z were against") and never "X and others" across positions, keeps
    similar things apart by maker, date or name, and cites each named person's
    own message; merging never moves a person or folds positions. The final
    prompt says the same in one sentence. `prompt::VERSION` 4: every cached
    note is taken again.
  - **Citations capped.** `[#a][#b][#c]…` with only spaces between is one
    citation, written `[#a, #b, #c]`: unknown ids out, repeats out, then the
    first three.
- **ReVision's server is reported, and can be stopped (2026-10-15, the
  account holder's ask).** While the view is open and nothing of ours is on
  the port, it looks every 5 s at who listens there (`tt-app/src/holder.rs`:
  `GetExtendedTcpTable`'s owning PID, `QueryFullProcessImageNameW`, `/props`)
  and says so under the form: whose (ReVision's by its `ReVision-data` path),
  which model, and what it costs — photos unread with Gemma, no summary at all
  with another model. **Stop it**, pressed twice, ends the process; only a
  `llama-server.exe`, re-checked by PID and path first. A run never stops it:
  it skips the photos, says so, and the finished status counts them.

## Decided (2026-10-14)

| question | answer |
|---|---|
| where it lives | **its own rail view**, "Summaries" |
| output language | **chosen per summary**: Serbian (Latin, ekavian) / English / same as the chat; remembered |
| the model server | **attach, else start its own**: use a Gemma already on the port (ReVision's included); if none, start ReVision's `llama-server` on ReVision's Gemma (or what Settings names) and unload it after 2 min idle; never stop a server it did not start |
| length | **Short** (one paragraph) · **Detailed** (prose, one theme per paragraph, most important first) · **Sections** (Key events, Decisions, Plans & deadlines, Disagreements, Open questions, bullets) |
| scope | **Whole group** or **By topic** (forums only); a single topic can also be picked |

## The view

```
┌ Chats ───────────────┐┌ KRGM ─────────────────────────────────────┐
│ ▸ KRGM               ││ From [2024-12-01]  To [2024-12-07]        │
│   RGZM               ││ [Yesterday] [Last 7 days] [This month]    │
│   UA KOLAB           ││ Length  ( Short | Detailed | Sections )   │
│ ──────────────────── ││ Scope   ( Whole group | By topic ) [topic]│
│ Saved                ││ Language ( Serbian | English | As chat )  │
│  1–7 Dec · Detailed  ││ 3,412 messages · ~96K tokens · 5 parts    │
│  2 Dec · Short       ││ [ Summarise ]                             │
│                      ││ ───────────────────────────────────────── │
│                      ││ Part 3 of 5 · 41 tok/s                    │
│                      ││ The week opened with … [#18234] …         │
│                      ││ [Copy] [Save]                       Redo  │
└──────────────────────┘└───────────────────────────────────────────┘
```

- **Chats** are the databases in the Library's folder, `tga_run::refresh::library`;
  **Saved** lists the selected chat's `analysis\summaries\*.md`, newest first.
- The count, token estimate and part count update as the range changes, **before
  the model is involved**, so a month of a 300k-message chat is never a surprise.
- Text streams in as the final call writes it. A citation `[#id]` opens the
  Viewer on that message (`viewer_open(db, Some(id))`); ids outside the range are
  dropped before display.
- **Save** writes `<chat folder>\analysis\summaries\<from>_<to>_<mode>.md`, with a
  header: chat, range, scope, length, language, model file, message count, when
  written. Opening a range already saved shows it at once, with **Redo**.

## The pipeline

**Input is the digest.** `tga_run::cli::digest_rows` already turns an export
into model-facing rows (id, time, topic, who, text, reply, media, document
text) with the report's own names. Filter by `Msg::when` (wall clock: a day is
a calendar day), `from 00:00` to `to 23:59` inclusive, and topic; read
attachments only for messages in range.

**One line per message**, a header at each day:

```
== 2024-12-02 (Mon) ==
#18234 09:14 Ana: …
#18240 09:20 Marko (re #18234): … [photo]
#18251 10:02 Ana: [doc "Zapisnik 01.12.docx", dated 2024-12-01: …first 1500 chars…]
```

Gemma already reads this shape in ReVision.

**Fitting 32K of context.** Token counts come from the server's `/tokenize`, so
cuts are exact. Budget per call: context − system prompt − answer reserve
(Short 600, Detailed/Sections 2,500).

1. Fits in one call: one call.
2. Does not: **map-reduce.** Cut parts at day boundaries, or at the longest quiet
   gap inside a day that is too big alone; never inside a message. Each part →
   **notes**: JSON-schema'd facts, each with its message ids, capped with
   `maxItems` (Gemma loops in id lists without caps). Then one final call writes
   the Short/Detailed/Sections text from the notes, streamed.
3. Too many notes for one call: merge them in tiers first.

**By topic** runs that once per topic. Topics under a handful of messages in
range fold into one "other topics" line; Detailed and Sections get a short
overview on top.

**Sampling:** Gemma's own (temperature 1.0, top_k 64, top_p 0.95), thinking
off. A note answer that is not valid JSON is retried (three attempts, as
ReVision's `run_local.py`).

**Cache:** each part's notes are stored under a hash of the part's rendered
text + prompt version + model file, in `<chat folder>\analysis\summaries\cache\`.
A sync that changes a message changes only its part's hash; overlapping ranges
reuse each other's parts.

## The model server

Settings › Summaries: the models folder, the model (a dropdown of the
folder's), `llama-server.exe` (blank: the one in the folder), port (8080),
arguments, idle unload (2 min). See `docs/MODELS.md` for what goes in the
folder.

On Summarise:

| on the port | then |
|---|---|
| healthy, serving a Gemma or the chosen file (`/props` names the model) | use it; never stop it |
| healthy, serving something else (e.g. another program's OCR model) | say which, do nothing |
| nothing | start ours (`CREATE_NO_WINDOW`, absolute path, output to `llama-server.log` beside `tgx.log`), wait for `/health` with a timeout, show "Starting Gemma…" |

Ours stops after the idle time and when the window closes. Gemma takes 15.1 GB
of the 16 GB card; desktop apps holding GPU memory push it to RAM and make it
~2.5× slower. The status line shows tokens/s so that is visible, and the view
says to close apps rather than changing the context size.

## Privacy

- **Loopback only.** The client refuses any host but `127.0.0.1`, `localhost`,
  `::1`. Chat text never leaves the machine. CLAUDE.md's "the analyser never
  touches the network" becomes exactly that sentence.
- `tgx.log` gets counts, parts, token figures and timings, **never text** — held
  by a test, like `a_detail_line_never_carries_the_message_text`.
- Summaries sit beside the database in `Exports\`, which already carries the
  same conversation; they inherit its `.gitignore` line.
- Live checks by Claude look at timings and counts only, not at summaries or
  messages, unless the account holder approves a specific chat.

## Layering

```
tga-sum      prompts, line rendering, cutting, notes/final calls, the cache,
             a loopback HTTP client on std::net (no new dependency), and the
             llama-server lifecycle. Depends on tga-notes (Row) only.
             MUST NOT depend on tga-read, tga-db, tga-docs, grammers-* or a toolkit.
tga-run      summary.rs: load the chat, build and filter rows, drive tga-sum,
             report Progress over mpsc + Wake (the Library's pattern).
tt-app       shell/summaries.rs: the rail view.
```

`layering.rs` gains the `tga-sum` rules; no `tgx-*` crate reaches it.

**CLI**, for judging prompts before any UI and for agents:

```
TelegramTools summarise <db> --from 2024-12-01 --to 2024-12-07
        [--short|--detailed|--sections] [--by-topic] [--topic T]
        [--lang sr|en|chat] [--out P]
```

## Phases

| # | what | checked by |
|---|---|---|
| 1 | `tga-sum` core + `summarise` CLI (attach-only) | unit tests: range edges, cutting, budget, citation check, cache key; a fake llama-server in-test (`TcpListener`, canned `/tokenize` and SSE) runs the whole pipeline without a GPU, including bad JSON, timeout, cancel; then **the user judges quality** on a real chat and the prompts are tuned |
| 2 | server start / idle unload / Settings fields | fake-server tests; one live start and stop |
| 3 | the rail view: picker, estimate, progress, streaming, citations into the Viewer, Copy/Save, saved list, cache | `save.bat test`; the user tries it |
| 4 | CLAUDE.md, `docs/AI.md`, layering rules | — |

Prompt quality is the real risk, which is why phase 1 has no window.

## To measure early

- `tga_db::load` reads the whole chat. Time it on KRGM (333,582 messages); if it
  is seconds, keep the loaded export between summaries of the same chat, and if
  it is worse, add a date-bounded read.
- Gemma's real throughput here: prefill and generation tok/s per part, to make
  the estimate line honest.

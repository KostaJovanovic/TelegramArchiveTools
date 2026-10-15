# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repository.

**TelegramTools**: two programs becoming one, in one workspace.

- **The exporter** (`tgx-*`) gets chats off Telegram. It reproduces **Telegram
  Desktop's own export format byte for byte**, plus the one thing Desktop
  cannot do: forum supergroups split into one folder per topic. It can also
  keep each chat as an accumulating `<chat>.sqlite`.
- **The analyser** (`tga-*`) reads a finished export, a folder or a database,
  and writes one self-contained `report.html` beside it. It never touches the
  network — except that a **summary** (`tga-sum`) talks to a model server on
  `127.0.0.1`, and only when asked for one.

They were two repositories until 2026-10-12; `TELEGRAMTOOLS.md` is the merge
plan and its log, and says which phase is done. They ship as one binary,
`TelegramTools.exe`: no arguments open the window, `analyse` runs the
analyser's command line, and anything else is one of the exporter's commands
(`login`, `chats`, `export`, …) — what used to be `tgx.exe`.

`ROADMAP.md` holds the exporter's design record and what is still open.
`docs/analyser/PLAN.md` is the analyser's: why each figure is computed the way
it is. `docs/analyser/REFACTOR.md` is the log of the rewrite that gave the
analyser its current shape. `AUDIT.md` holds
what past reviews and live runs found. `DEFECTS.csv` is the register of known
output differences against Desktop, with a `caught_by` column naming what would
have found each; its `status` column says which are open, and several are
invisible to every leg. Read
the relevant part before changing the area it covers.

## Commands

```powershell
save.bat                    # menu; also takes an action as an argument
save.bat test               # fmt + clippy + every suite, same as CI
save.bat build              # release build -> dist\TelegramTools.exe
save.bat parity             # all four replay legs against corpus or drive
save.bat corpus             # cut reference\ out of the reference export
save.bat wire <export dir>  # diff a live export against the reference run
save.bat baseline           # the analyser's reports against the last recording
save.bat baseline record    # re-record it, after reading the diff
save.bat bless              # re-record the analyser's committed golden
save.bat report <folder>    # write report.html beside an export
save.bat run                # the window, from source
save.bat clean              # report target\ and empty it
save.bat save|commit        # test, then commit (+ push for save)
save.bat release            # test + commit + push + tag v<Cargo.toml version> + build
save.bat push|pull          # current branch; --force = force-with-lease + if-includes
```

`save`, `commit`, `release`, `push` and `--force` are git actions: they commit
or publish, so don't run them unless asked.

Underneath it is plain cargo:

```powershell
cargo test --all
cargo test -p tgx-html                          # one crate
cargo test -p tgx-tg --lib config::tests        # one module
cargo test -p tgx-html --test preview_parity    # one integration suite
cargo test -p tgx-tg the_last_target_wins       # one test by name substring
cargo clippy --all-targets --all-features -- -D warnings
cargo run -p tt-app                              # the window
cargo run -p tt-app -- login|chats|export "<title>"
cargo run -p tt-app -- export --db "<title>"     # sync the database
cargo run -p tgx-parity -- archive reference     # one leg on its own

cargo test -p tga-report --test golden          # the analyser's golden; no corpus needed
cargo test -p tga-db --test corpus -- --nocapture   # every database on L:
cargo run -p tt-app -- analyse <folder or .sqlite>
        [--out P] [--chat ID|TITLE] [--digest] [--handles] [--no-fonts]
        [--stats P] [--notes P] [--stamp T]
cargo run -p tt-app -- analyse <output folder> --refresh [--force]
cargo run -p tt-app -- analyse --from-stats <s.json> --out <p>
cargo run -p tt-app -- summarise <folder or .sqlite> --from D [--to D]
        [--short|--detailed|--sections] [--by-topic|--topic T] [--lang sr|en|chat]
cargo test -p tga-sum --test fake_server        # the whole summary pipeline, no GPU
```

A release build is a GUI-subsystem exe, so **cmd.exe does not wait for it**:
run an interactive command as `start /wait TelegramTools login`. PowerShell
waits on its own. The output still reaches the terminal — `main` attaches to
the parent's console.

**Pass `-- --nocapture` to anything touching the analyser's corpus tests.**
Their skips are `eprintln`s, and libtest discards stderr for a passing test, so
without it a suite that compared nothing reports as coverage. `save.bat` and CI
both pass it.

CI (`windows-latest`) runs fmt, clippy with `-D warnings`, `cargo test --all`,
a release build, and fails if the binary exceeds 30 MB. Every resolving step
passes `--locked`, so a dependency change must ship with its `Cargo.lock`.

`save.bat clean` exists because `target\` is a cache cargo never collects; it
reached 45 GB once. `[profile.dev.package."*"] debug = false` in the root
`Cargo.toml` handles the recurring half — do not remove it.

## The oracle

**Nothing here is verified by unit tests alone.** The output is checked against
a real Telegram Desktop export of the same chat, replayed through our own
writers and diffed. `crates/tgx-parity` is that harness.

| leg | direction | current |
|---|---|---|
| `json` | reference `result.json` → our emitter → byte diff | 4/4 topics, 6,643 messages |
| `html` | reference `result.json` → our writer → line diff vs Desktop's pages | 4/4 topics, 256,780 lines |
| `media` | reference `result.json` → our name planner → diff the tree | 830/836 (ceiling: custom emoji) |
| `archive` | reference `result.json` → two merges into the store → read back → byte diff | 4/4 topics, 6,643 messages |
| `wire` | our own live export vs a reference run | run once, 2026-08-27; see `AUDIT.md` |

The reference export is `N:\telegram export\UA KOLAB TELEGRAM`. No leg reads
media — the media leg diffs *names* against the tree the JSON records — so
`save.bat corpus` cuts only the 7.8 MB text half into `reference/`, where
`crates/tgx-parity/tests/corpus.rs` runs all four replay legs as an ordinary
`cargo test`, sha256-checked against `MANIFEST.txt`.

`reference/` is gitignored: it is verbatim chat history from real people and
this repo has a CI workflow. Without it the corpus test skips, so:

- `save.bat` sets **`TGX_REQUIRE_CORPUS=1`**, turning a missing corpus into a
  panic. `set TGX_REQUIRE_CORPUS=0` opts back out.
- CI runs `cargo test --all -- --nocapture` and raises a `::warning::`.
- A plain `cargo test --all` still skips silently, so a fresh clone can run the
  suite.

A corpus cannot be trimmed to "a few interesting messages": pagination and
message joining are cumulative down a topic, so page 3 only reproduces if pages
1 and 2 were written first. Whole topics or nothing.

**If a diff fails, fix the code, not the reference.** Editing the reference to
agree with us turns the oracle into our own output wearing Desktop's name.

## Architecture

Layered, enforced by `crates/tgx-parity/tests/layering.rs` rather than by
convention:

```
tgx-format   Desktop's JSON schema, key order, escaping, sizes. No I/O, network or UI.
tgx-html     the pages, written from serialised maps. MUST NOT depend on grammers-*.
tgx-media    classification, folder layout, filenames, stripped thumbnails.
tgx-archive  the database output: one <chat>.sqlite per chat. MUST NOT depend on grammers-* or tgx-tg.
tgx-tg       grammers client, topics, engine, planning, download, enrichment.
tgx-ui       the design system in egui: tokens, type scale, components, theme.
tgx-parity   the oracle: lib + bin. Depends on format/html/media/archive, never on tgx-tg.

tga-read     export folder -> model. Both layouts: ours (result.json per topic at
             the root) and Desktop's (chats/chat_<id>/result.json). No UI, no network.
tga-db       <chat>.sqlite -> the same model, read through tgx-archive's read-only
             Store. Hands the stored JSON to tga-read rather than parsing it again.
tga-docs     a .txt/.docx/.pdf attachment -> its text and its real date.
tga-notes    the hand-written annotation layer + the model-facing digest.
             MUST NOT depend on tga-read or tga-docs.
tga-stats    the shape of every figure, and the dump's format. Data only.
tga-metrics  every figure, one pass. No I/O. Fills in a tga_stats::Stats.
tga-report   the ramp, the SVG marks, the one HTML file. Reads a Stats.
             MUST NOT depend on tga-read or tga-metrics.
tga-sum      a local model's summary of a date range: digest rows in, text out.
             Line rendering, cutting into parts, prompts, notes cache, a loopback
             HTTP client on std::net, the llama-server keeper. Reads no export.
             MUST NOT depend on tga-read, tga-db, tga-docs, tga-metrics or a toolkit.
tga-run      running an analysis: the `analyse` and `summarise` command lines,
             the library of exports, the workers. No toolkit. The only crate
             that reaches tga-docs and tga-db.

tt-app       TelegramTools.exe: the window, and both command lines behind its
             argv. Depends only on tgx-ui + tgx-tg + tga-run.
```

The analyser's rules sit in the same `layering.rs`: no `tga-*` crate reaches
`grammers-*`, `tgx-tg` or `tt-app`, `tga-run` has no window in it, and **no
`tgx-*` crate reaches any `tga-*`**, so the exporter's oracle cannot move
because a report changed.

A Telegram type in `tgx-html` would let Desktop's markup depend on wire shapes,
and the harness could no longer replay recorded JSON through it. That is the
whole reason the rule exists, and `tgx-archive` is there for the same one: the
archive leg merges a real export into a store twice and reads Desktop's own
bytes back, which is only possible offline.

**Both outputs come from one map.** `tgx-tg/src/output.rs` takes the payload
that goes into `result.json`, strips the presentation-only `_p` key, and hands
the whole map to the HTML writer. The two cannot drift, and the writer stays
testable with no connection — which is what makes the html leg possible.

**Classic or Database — the two are exclusive.** `Settings::writes_folders()`
is the single place that choice becomes "is there a folder", so the engine, both
front ends and the "nothing to write" guard cannot disagree. Classic writes
Desktop's folders and re-reads the chat from the beginning every run, because it
produces a complete standalone export. Database writes one accumulating
`<chat>.sqlite`, media inside, and syncs: what is new plus `reread_window`
(default 500), with `reread_all` for the whole pass.

Running both was the first design and it is the wrong one: Classic's full
re-read happens anyway, so the pair costs exactly as much as Classic and is
incremental in name only.

**One database per chat, named after the chat, in a folder of its own.**
`ChatExporter::open_archive` opens `<output_dir>/<title>/<title>.sqlite`
(sanitised) at the top of every `run`; `archive_path` moves an older flat
`<output_dir>/<title>.sqlite` (and its `-wal`/`-shm`) into that folder. The
assignment drops the previous chat's connection. The name has to be *stable* or
a re-run would start a fresh archive instead of syncing, so it is deliberately
not `unique_dir` — no `(2)` on a clash. The clash is caught inside the file
instead: `Store::holds_another_chat` asks whether the `chats` table already
names somebody else, and if it does the chat id joins the folder and filename, on this run
and on every later one. `chats` and not `messages`, because a chat is recorded
before its first message and a chat that exported nothing still owns the file it
made. A store that will not open now fails **that chat**, not the queue — with a
file each, the next chat has every chance of working.

**The database is fed the same map the writers are**, minus `_p`, and renders
nothing.

**A windowed run is not a short run.** It reads the newest few hundred messages
on purpose, so it publishes no `Progress::Total`, sends `expected: 0` and
`messages_measured: false` on `ChatDone`, and is judged complete by
`reached_end` rather than by a count. Do **not** write `ExportResult::complete`
as `reached_end || <the count rule>`: `Ok(None)` is how the read loop ends on
nearly every export, so that `||` silently removes the INCOMPLETE warning from
all of them.

**`merge_volatile` decides what a re-read may overwrite.** Ten volatile keys —
the text, the counters, the reactions — plus any media key whose stored value is
one of Desktop's `"(File …"` placeholders, which records a refusal to fetch
rather than what the message carried. Everything else is left alone, so a later
converter fix never reaches a stored message (in ROADMAP).

**A file is fetched when the archive lacks the bytes, never when the message is
new.** `worth_fetching` gated on `Merge::Inserted` first, which meant anything
missed on the run that first saw a message — media off, over the size limit, a
failed download — was missed for good. A sync backfills, but only for messages
it re-reads, so the window bounds how far back that reaches.

**One pass per chat, oldest first.** `engine.rs` uses
`iter_messages(peer).reverse(true)` with a resume loop keyed on `offset_id`,
routing each message to its topic's `Output` as it arrives. Do not convert this
to per-topic thread fetches: `messages.getReplies` returns nothing for the
General topic, so that silently loses it and multiplies requests by the topic
count.

**Closing drains, and the database commits in the same place.** `close_all` is
the one point every exit from `run` already passes through, so the batch commit
lives there too — the store has no `Drop` backstop that could cover a missed
site, because a `Drop` cannot await. The JSON is streamed, so a run abandoned
without `close()`
leaves a file that is not truncated but **zero bytes**. Every path that can end
an export goes through `Output::close`, with a `Drop` impl as backstop.

**egui on the main thread, tokio on its own.** `tt-app/src/bridge.rs` is the
seam; shutdown drains, it does not stop.

**A worker event repaints the window.** Otherwise a result sits in the channel
until something else causes a frame, which the user experiences as having to
move the mouse to make the app work. `bridge::Events` is what guarantees it: it
wraps the sender and calls `ctx.request_repaint()` after every send, so no call
site has to remember to. `Shell::update` drains the whole batch at the top of
each frame and rebuilds the rows once.

The rule predates egui. Under GPUI the mechanism was the opposite — the channel
had to be *awaitable*, `Bridge::drain` was `#[cfg(test)]` so a poll could not
come back, and `shell/mod.rs` awaited on the foreground executor. Polling from a
frame is now the only way to read a channel, and `Events` is what makes it
correct. **Do not add a send path that bypasses it.**

**One connection for the whole process.** `Session::connect` hands out a handle
on a shared `Connection` (`tgx-tg/src/client.rs`); it does not open one.
Per-action sessions each paid TCP, an `InvokeWithLayer(InitConnection)` and a
write of every datacentre address before their first useful request, and opened
a second `SqliteSession` on a file the first was still writing.

**Every network step has a timeout,** because no layer underneath has one:
`NetStream::connect` in grammers is a bare `TcpStream::connect`, so a filtered
address sits in the OS's SYN retry (~21s on Windows) with the UI reading
"Working…". `CONNECT_TIMEOUT` is deliberately under that; `AUTH_TIMEOUT` is
longer because `auth.sendCode` can contain a whole second connection
(`PHONE_MIGRATE` means a fresh DH exchange against the home DC first).

**A Telegram server error is sent again, not believed.** `client::Patient` is
the client's retry policy: grammers' own `AutoSleep` for flood waits, plus three
re-sends (2, 4, 8 s) for the 500 family and -503 `Timeout`, which
`classify` calls `EnrichError::Unavailable`. grammers' default gives up on
those at once, and one `RPC_CALL_FAIL` ended a 122,487-message chat at 48,312.
The read loop resumes on one that outlasts the policy, against
`MAX_STALLED_WAITS`.

**Anything needing an authorised account calls `ensure_connected` first**
(`actions::ready`). It bounds a blocked network and names an unauthorised
account instead of surfacing either as a wire error mid-listing. Cached on the
connection: one round trip per process, not per action.

**One writer per fact.** A chat's message count has three sources and one
setter, `Shell::set_count`. The queue owns what a run did, so no worker composes
its own summary. The second writer always wins a race you did not know you had.

## The analyser

Point it at a finished export, folder or database, and it writes one
self-contained `report.html` beside it. In the window it is the **Library**
view (`tt-app/src/shell/library.rs`): every export under a folder is a row,
with what needs a report already ticked. The folder it opens on is the
exporter's destination, and it lists the Classic folders there one per chat
beside the databases. A row's **Read** is the way into the Viewer, which is
not on the rail; a summary's citation is the other, and its way back returns
to Summaries (`ViewerState::back`). "Report after export" hands each finished chat to it
(`Library::report_later`). Its workers report on their own `mpsc` channels, not
the bridge's, and wake the window through `tga_run::job::Wake` — a callback,
so `tga-run` has no toolkit in it.

**`tga-report` renders from a `tga_stats::Stats`, which is exactly what
`--stats` dumps.** That is what lets a recorded dump replay through the writer
with no export on disk; the golden test and `--from-stats` both rest on it. The
shape lives in `tga-stats` so the writer depends on neither the reader nor the
metrics. `tga-notes` inherits the rule at one remove, so the
`Export -> digest::Row` and `Doc -> Attachment` mappings live in the caller
(`tga-run`), not in `tga-notes`.

Every field of `Stats` has a `Default` and the struct is `#[serde(default)]`,
so a dump with a branch missing renders an empty section. `dynamics` is an
`Option` on purpose: absent means "recorded before this existed", which is a
different fact from an archive with nothing in it.

### Invariants that change numbers or break the harness

- **Time is two clocks.** Anything with a calendar or clock face reads
  `Msg::when` (naive local wall clock); anything measuring a *duration* reads
  `Msg::unix`, which stays monotonic across DST. Decided once in `tga-read`.
- **A deleted message still counts.** `tga-db` does *not* filter on
  `deleted_seen`. Keeping what was deleted is the whole reason the database
  format exists; the count is reported separately instead.
- **A document has its own date, and it is not the message's.** `tga-docs` dates
  an attachment from its filename and text, never the filesystem, and names the
  rung it stopped on (`filename`, `content`, `filename+posted`, `posted`).
- **A sender is a typed peer key, never a display name.**
- **A forum topic is a thread**, so the reply to the topic's root is dropped.
  Read literally, response-time medians stretch to days.
- **The stats dump is sorted by `tga_stats::write`, not by the map type.** In
  this workspace `serde_json` has `preserve_order` on (the exporter needs it),
  so `Map` keeps insertion order and the explicit sort is the only thing
  ordering the dump. Do not remove it.
- **`assets/report.css` and `report.js` are copied into the report verbatim.**
  Editing either changes every report's bytes. `.gitattributes` pins them and
  the goldens as `-text`; a CRLF checkout of either fails every compare.
- **`Count` serialises as `["label", 41]`**, not as an object. Changing it
  invalidates every recorded dump.
- **Nothing in a notes file is trusted.** Every string is escaped, and a
  malformed entry is dropped, never raised.
- **One file, no network.** Fonts base64'd, charts inline SVG, every view
  rendered server-side; JS only filters and pans what is drawn.
- **Reading a database writes nothing into it.** It is opened through
  `Store::open_read`. SQLite still creates an empty `-wal` and a `-shm` index
  beside a WAL database, even for a reader, and the old rusqlite reader did the
  same; what the test holds is that the database's bytes and its log are
  unchanged.

### The analyser's oracle

**`save.bat baseline` is the load-bearing check.** It runs the release build
over the real archives and compares the whole report and the whole stats dump,
byte for byte, against a recording in `baseline\`. A difference is a mistake
unless the change was meant to alter the output; then it is
`save.bat baseline record`, after reading the diff. Three legs: `ua-kolab`,
`krgm`, and `krgm-notes`, which passes the 42 hand-written notes.

| corpus | messages | what it is for |
|---|---:|---|
| `N:\telegram export\UA KOLAB TELEGRAM` | 6,643 | baseline; also cut into `reference\` |
| `J:\temp pureraw\KRGM*` | 333,582 | baseline |
| `N:\telegram_export\KROVNA RADNA GRUPA ZA MEDIJE*` | 450,817 | `tga-docs` corpus: 150 docx, 238 pdf, 9 txt |
| `L:\9 telegram export\` | 1.6M in 63 databases | `tga-db` corpus: every database must read |

The drive letters move: on 2026-10-12 `N:` was a different disk, and only `L:`
held its corpus. **`tga-read`'s corpus test falls back to `reference\`**, the
exporter's cut of the same UA KOLAB export, so it runs without the drive.

`TGA_REQUIRE_CORPUS=1` (set by `save.bat`) turns a missing corpus into a
failure. `TGA_EXPORT`, `TGA_DOCS_EXPORT`, `TGA_DATABASE` and
`TGA_DATABASE_OUTPUT` override the paths. `TGA_BLESS=1` re-records the golden.
The document leg takes a minute in a debug build, because it parses 238 PDFs.

**`tools/merge_check.ps1`** records what the analyser writes, the digest and a
hash of every field of the database model included, and is how the merge was
proved to change nothing. Run it before and after any change that could move
the reader or the serialiser, and compare the two `MANIFEST.txt` files.

**Two traps in the document corpus:** the `.txt` files at each topic's root are
`missing_media.txt`, the exporter's own log, not attachments; and 133 of the
referenced `.docx` were never downloaded, so a document that reads as empty is
a missing file far more often than a broken parser.

The committed fixtures (`tga-report/tests/fixture.stats.json`,
`tests/synthetic/`) are hand-written and name nobody, which is why they can be
committed and never skip.

### Summaries

The **Summaries** view (`tt-app/src/shell/summaries.rs`) and `TelegramTools
summarise` ask a local model what happened in a chat between two days.
`docs/SUMMARIES.md` is the design record. What must hold:

- **Loopback only.** `tga_sum::Client` takes a port, never a host, and
  `server::arguments` drops any `--host`/`--port`/`-m` from the user's server
  options. The summary sends other people's conversation to the model, so the
  model is on this machine.
- **The narration never carries a message.** `Event::Say`, the status line and
  `tgx.log` name counts, parts and timings; `the_narration_never_carries_a_message`
  holds it.
- **Input is the digest's row.** `tga_run::cli::digest_row` is shared with the
  digest — same names, same document reading — with the text kept whole; the
  range is picked by `Msg::when`, so a day is a calendar day.
- **Independent of ReVision: one models folder.** `summary_models` (default
  `Models\` beside the exe, gitignored; kept by Reset) holds the `.gguf` files
  and llama.cpp's `llama-server.exe`, each in it or one folder down
  (`server::server_in`, `server::models_in`). Settings › Summaries chooses the
  folder, lists its models in a dropdown (`summary_model`, relative to the
  folder; empty or gone → `server::default_model`, the first Gemma), and
  carries a short *How to add a model* whose long version is
  `docs/MODELS.md`. Keep those two in step. Nothing looks for ReVision's
  folder any more (2026-10-15, the account holder's ask, which reverses the
  earlier one to share ReVision's). Its numbers stay: the options and sizes
  were measured there.
- **Attach before start; never stop what we did not start — unless asked.** A
  Gemma, or the chosen file, already on the port is used; otherwise `Keeper` starts
  `llama-server` by absolute path with `CREATE_NO_WINDOW`, and the window
  stops it after `summary_idle_minutes` **once the Summaries view is left**
  (a citation's trip to the Viewer is not leaving; while the view is open
  `summaries_idle` keeps touching it) and on exit. Two copies of a 15 GB model
  do not fit the 16 GB card. Somebody else's server on the port is looked at
  every 5 s while the view is idle (`tt-app/src/holder.rs`: the TCP table's
  owning PID, its image path) and **reported** under the form — what it has
  loaded, what that costs — with **Stop it**, pressed
  twice. Only a `llama-server.exe`, re-checked by PID and path before
  `TerminateProcess`. Nothing stops it on its own.
- **Photos are read first, by PaddleOCR-VL** (`tga_sum::ocr`, from the
  models folder or a folder of its own in it, with ReVision's profile and
  prompt, its Cyrillic fix and loop cut). The card holds one model, so `Keeper::ensure(.., Want)` stops
  the server *it* started when the other model is wanted: OCR, then Gemma.
  Readings are kept in `cache\photos.json`, keyed by the OCR's file and the
  picture (`kind:file_id`, or a folder export's path and size), and land in
  `Row::photo_text`, which the digest never fills. Another program's Gemma on
  the port means the photos are skipped and said, never that its server is stopped. Only
  JPEG and PNG are sent: no image decoder is linked, so WebP is skipped and
  nothing is shrunk.
- **ReVision's sizes, not ours.** Raw messages are read in parts of
  `PART_TOKENS` = 4,500 (ReVision's `CHUNK_TOKENS`, where its extraction found
  79% of events, not 47%), and a summary reads raw messages in one call only up
  to that size (`SINGLE_TOKENS`); any other call reads at most `INPUT_TOKENS` =
  14,000 (its `INPUT_CAP`). Messages are cut at 1,500 characters, and "haha",
  "ok" and a lone sticker are dropped unless answered (`lines::trivial`). These
  were 8,000/16,000/1,200 and kept, until a summary skipped a discussion that
  mattered. Change one and say why, against a measurement.
- **A conversation rests on the same reading.** The question box under the
  summary (`tga_sum::ground` + `answer`, `tga_run::summary::ask`) reads the
  form's days whole up to `INPUT_TOKENS`, else the summary's own cached notes,
  and keeps that `Grounding` for every question about the same days. A
  question's words never reach the log, only its length.
- **Over notes, a question reads the messages.** A note can be wrong about who
  held which position, and that is what questions ask. So `answer` first has
  the model pick the notes the question turns on (`prompt::pick`, JSON), then
  reads those notes' messages ±3 (fewer, then fewer notes, until it fits) and
  each one's reply parent, and may cite only those. Reading all the notes is
  the fallback when nothing is picked, not the plan.
- **Every citation is checked** against the ids the model was given
  (`cite::Filter`, on the stream). A citation to the wrong message is worse
  than none. Brackets side by side are one citation, kept to `cite::MOST` = 3
  ids; the stream holds a finished citation back until the next character
  shows whether another joins it.
- **Notes are cached by what the model read** (`cache::key`: prompt version,
  model, language, the part's text). Change a prompt in `prompt.rs` and bump
  `prompt::VERSION`.

No test here judges whether a summary is *good*; the fake server
(`tga-sum/tests/fake_server.rs`) proves the plumbing only. Quality is judged by
the account holder on real chats — and the summaries, like the chats, are not
for Claude to read unless they approve that chat.

## Byte-exactness invariants

Each was found by diffing, not by reasoning:

- `result.json` has **no trailing newline** — Desktop ends on `}`.
- 1-space indent, raw UTF-8, and a deliberate **over-indent on `reactions`**.
- Key order comes from `tgx-format/src/order.rs`; the json leg asserts that
  re-keying a real export is a no-op.
- Media folder follows the file's **shape**, not its `media_type` — a WebM video
  sticker goes to `video_files/` while still reporting `"media_type": "sticker"`.
  Routing on `media_type` alone shifts every later filename in the folder.
- HTML: closing tags return to the opening tag's indent; the doctype is emitted
  as text, not a tag line; attributes alphabetical.
- A message whose text ends on an entity gets **a trailing empty segment, but
  only when the text is not pure ASCII** (`tgx-format/src/text.rs`). It is the
  signature of a UTF-16 end offset compared against a UTF-8 byte length; for
  ASCII the two agree and Desktop emits nothing. The entity type plays no part.

## What no test here can catch

The four replay legs prove everything *downstream* of the wire and open no
sockets. `convert.rs` and `plan.rs` — TL object in, Desktop JSON out — have only
synthetic fixtures (`crates/tgx-tg/tests/`), because the reference records
Desktop's output, not Telegram's input.

The archive leg is downstream of the wire too, and downstream of the *converter*
as well: it replays recorded payloads. So it says nothing about whether the
engine hands the store the right payload, whether the deletion sweep runs only
after a finished walk, or whether the media ingest picks up the file that was
actually written. Those have synthetic tests and a live check, like the rest of
the wire.

This is not theoretical. `Session::connect` once took `SenderPool`'s handle and
dropped its runner, so every request was cancelled before it was sent, with all
tests passing and all three legs green. Separately, four whole features —
`reactions`, service `action`, polls, locations — were emitted by nothing at all
under 444 passing tests, because a key the converter never writes is a key the
reference supplies for the legs. **Treat green suites as saying nothing about
the wire.**

The wire leg is the only check that can, and it tests for two absences as well
as for differences: a media path in our JSON with no file behind it (`dangling`)
and a field the reference writes that we never write (`absent`, kept outside the
`MAY_DRIFT` allowance). Nothing downstream of the converter can miss a key the
converter never emits.

## Paths, state and credentials

State lives **beside the executable** — never AppData, never the registry —
except for a binary under `target/`, which climbs back to the workspace root,
because `cargo clean` would otherwise delete the session key and every export.
`save.bat build` ships to `dist/`, so `dist/` can be copied anywhere and still
work.

`TelegramToolsData/` holds the `api_hash` and a Telegram **session key, which
is a bearer credential**: anyone who can read it can act as the account. It is
gitignored and ACL-restricted on creation (a protection that does not exist on
FAT32/exFAT). Do not run a live export or read stored credentials without the
account holder saying so explicitly.

It was `TelegramExporterData/`. `config::resolve_data_dir` renames the old
folder on the first run that finds it alone, and **keeps using it if the
rename fails** rather than starting an empty one — an empty folder would sign
the user out and strand their credential under a name nothing reads.

**`Exports/` is not protected, and the databases live there.**
`<output_dir>/<chat>/<chat>.sqlite` holds every message of that chat, every earlier
version, every participant and every media byte — other people's conversation,
in the folder most likely to be copied to a drive, in the file most likely to be
handed to somebody on its own. That is why it is one file per chat and not one
for the account: a file named after one conversation must not carry the others
exported beside it. They are covered by the `Exports/` line in `.gitignore` like
everything else there, and it is why `Settings::without_credentials` exists: the
run record would otherwise carry the `api_hash` and the account's phone number
out of the one directory that is ACL-restricted. A run that writes no folder
still writes these files, and a database-only run's scratch tree
(`.tgx-scratch/<chat>/<topic>/`) lives beside them for the length of the media
pass.

`ensure_data_dir` needs three guards that are easy to drop: `CREATE_NO_WINDOW`,
a once-per-process flag, and an **absolute `System32\icacls.exe` path** — a
portable exe invoking a bare name lets a planted `icacls.exe` beside it run with
the user's rights.

It also holds **`tgx.log`, and `tgx.prev.log` for the run before it**, written
by `tgx_tg::logging::init`, called first thing in both binaries' `main`. Until
it existed nothing installed a `log` implementation, so every `log::` call here
and inside grammers went nowhere. The first column is seconds since start,
because the question the file gets opened for is almost always "what took the
time". `RUST_LOG` is a **plain level name**, not env_logger's filter grammar.
`log`'s `std` feature is not a default and `set_boxed_logger` is gated behind
it; it is set in the workspace dependency, because whole-workspace feature
unification hides a per-crate omission.

**An export narrates itself at two levels.** `Progress::Log` is stage
commentary — settings in force, the count and where it came from, topic folders,
the roster and whether it was capped, each resume after a rate limit, the read
total against what Telegram promised, and per media batch the queue, throughput
and every file that did not arrive. It goes to `tgx.log` and the window's
transcript. `Progress::Detail` is one line per message and per file, goes to
`tgx.log` and the CLI **only**, and is emitted at all only under
`RUST_LOG=debug` (`engine::detail_wanted` gates the formatting so an ordinary
run does not pay for it). It never reaches the window because the transcript is
a 2,000-line ring whose purpose is that the INCOMPLETE warning can still be
scrolled to, and one chat of six thousand messages would flush it.

**A detail line reports the message's text *length*, never its text.** The log
sits beside the executable and an export is other people's conversation;
`a_detail_line_never_carries_the_message_text` is what keeps it that way.

## Dependencies

`winresource` is pinned **exactly**, not by caret: it is pre-1.0, so a routine
`cargo update` can change its interface with no code change of ours, and what it
breaks is a build script — the failure surfaces in a crate whose source did not
change. Bump it deliberately, on its own commit, with the parity legs green
either side.

`gpui` and `gpui-component` carried the same pin and the same warning, and went
with the UI swap. `eframe` takes a caret: it is past 0.1, ships a changelog and
moves on a schedule.

**`eframe` is `default-features = false` with `glow`, and it must stay that
way.** `wgpu` drags in a shader compiler for a window that is a table, a
transcript and a row of buttons; `default_fonts` would merge egui's own Ubuntu
and Hack in as fallbacks, so a missing glyph would render in a typeface nobody
chose. `tgx-ui` and `tt-app` both declare it and must agree.

`libsql` is pinned **exactly** at `=0.9.30`, `default-features = false` with
`features = ["core"]`. `grammers-session` already links that exact version for
its session file, behind its own `sqlite-storage` feature — so `tgx-archive`
compiles nothing new and the binary does not grow (9.9 MB before and after).
A *second* version would bring its own `libsql-ffi`, and the two would export
the same `sqlite3_*` symbols and fail at link time; **`rusqlite` is the same
clash under another name and must not be added.** Bump it deliberately, on its
own commit, in step with grammers' pin, with the parity legs green either side —
the `winresource` rule, for the same reason.

`egui_extras` is the egui project's own companion crate, held at the same
version as `eframe` and taken `default-features = false` — its image, SVG,
datepicker and syntect extras are each opt-in and none is wanted. It is here for
`TableBuilder`, which lays out the queue: four fixed count columns and one
stretch column naming the chat, with a floor under the stretch. Laying that out
by hand meant the same five-term subtraction written twice under a comment
asking the next reader to re-check the sum.

`tools/gen_jpeg_header.py` generates the 623-byte JPEG header baked into
`tgx-media/src/jpeg_header.rs`; `tools/extract_preview_samples.py` produced the
committed `preview_samples.json` fixture. Regenerate rather than hand-edit.

## Other agent configs

An OpenAI Codex config (`~/.codex/config.toml`) and a Gemini CLI config
(`~/.gemini/settings.json`) exist on this machine. To bring over MCP servers,
slash commands, subagents, skills or instructions, reply `/import` to see what
is importable, then `/import --yes=<digest>`. If `/import` is unavailable on
this surface, run `claude import` from a terminal.

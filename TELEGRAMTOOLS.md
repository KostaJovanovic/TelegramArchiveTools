# TelegramTools — the merge plan

TelegramExporter and TelegramAnalyser become one product, **TelegramTools**:
one repository, one Cargo workspace, one `TelegramTools.exe`, one window. The
same window gets a redesign, so it reads as one tool for one job — get a chat
off Telegram, keep it, understand it — and stops reading as two tools that
happen to look alike.

This file is the plan. Once the work starts it becomes the log, the way
`REFACTOR.md` did for the analyser.

---

## 1. Where things stand (surveyed 2026-10-12)

| | TelegramExporter | TelegramAnalyser |
|---|---|---|
| crates | `tgx-format html media archive tg ui app parity` | `tga-read db docs notes stats metrics report app` + a **verbatim copy** of `tgx-ui` |
| binary | `TelegramExporter.exe` 12.4 MB, plus the `tgx` CLI | `TelegramAnalyser.exe` 10.6 MB (window and CLI in one) |
| UI | egui 0.33 on glow, light + dark | egui 0.33 on glow, dark only |
| oracle | 4 replay legs on `reference/`, plus `wire` | golden, `save.bat baseline` (3 legs, byte for byte), 3 corpus tests |
| toolchain | 1.97.0 MSVC | 1.97.0 MSVC — identical |
| remote | `github.com/KostaJovanovic/TelegramExporter` | `github.com/KostaJovanovic/TelegramAnalyser` (**public**) |
| uncommitted | `engine.rs` (log line says why it re-reads) | 6 files, 129 lines of them in `tga-read` |

Things that already line up: `tgx-ui` hashes identical in both repos, same
`eframe`, `rfd` and pinned `winresource`, same release profile, the same
`[profile.dev.package."*"] debug = false`. The analyser was built to be laid
out "with the exporter's own pieces", so the merge is mostly mechanical.

### The two things that do not line up

**1. `rusqlite` in `tga-db`.** The exporter's CLAUDE.md forbids it: grammers
and `tgx-archive` link `libsql =0.9.30`, and a second SQLite exports the same
`sqlite3_*` symbols and fails at link time. This is a blocker, not a
preference. The fix is also the better design: `tgx-archive` already owns the
schema and already has a tested read-only open (`Store::open_read`, with
`a_read_only_store_reads_and_refuses_to_write`). `tga-db` becomes a thin
adapter over it — rows out of the store, the three keys put back, maps handed
to `tga_read::one` as now. One crate owns the schema, not two that agree by
luck. The open questions are that libsql is async, so `tga-db` needs a small
`block_on`, and whether the analyser's "no `-wal`/`-shm` appears" test still
holds under libsql's read-only flag.

**2. `serde_json`'s `preserve_order`.** The exporter turns it on, because
Desktop's key order is part of the byte-exact output. The analyser needs sorted
output and already **sorts explicitly in `tga_stats::write`**, because its old
toolkit flipped the feature on through unification once before. In one
workspace the feature is on for every crate. The stats dump is protected. Any
other place where `tga-*` serialises a `Value` it built itself is not: the
digest, the notes and the report's embedded data. `save.bat baseline` catches
the report and the dump. **Nothing catches the digest**, so phase 0 records
it.

---

## 2. Architecture after the merge

```
tgx-format    Desktop's JSON schema, key order, escaping.          (unchanged)
tgx-html      the pages. MUST NOT depend on grammers-*.             (unchanged)
tgx-media     media classification and names.                       (unchanged)
tgx-archive   the database: writer AND the one reader.              (+ read API for tga-db)
tgx-tg        grammers, engine, planning, download.                 (`tgx` bin moves to tt-app)

tga-read      export folder -> model.                               (unchanged)
tga-db        <chat>.sqlite -> the same model, THROUGH tgx-archive. (rusqlite removed)
tga-docs      attachment -> text + real date.                       (unchanged)
tga-notes     annotations + digest.                                 (unchanged)
tga-stats     the shape of every figure.                            (unchanged)
tga-metrics   every figure, no I/O.                                 (unchanged)
tga-report    the one HTML file, from a Stats.                      (unchanged)
tga-run       NEW: tga-app's cli/refresh/job/state, without egui.   (library)

tgx-ui        the design system. ONE copy.                          (redesign lands here first)
tt-app        NEW: TelegramTools.exe — the window, and every command line.
tgx-parity    the oracle.                                           (unchanged)
```

**Crate prefixes stay as they are.** Renaming `tgx-*` to `tt-*` touches every
`use` line, every doc and the layering test, and buys nothing. Only the new
binary crate gets the new name.

**`layering.rs` grows the analyser's rules**, which are prose today:

- `tga-report` must not depend on `tga-read` or `tga-metrics`.
- `tga-notes` must not depend on `tga-read` or `tga-docs`.
- `tga-docs` is reached by `tga-run` only.
- **No `tga-*` crate depends on `grammers-*` or `tgx-tg`.** The analyser
  works on an export with no network, ever.
- **`tgx-*` export crates never depend on `tga-*`.** The exporter's oracle must
  not move when the analyser does.
- `tt-app` depends on `tgx-ui`, `tgx-tg` and `tga-run`, and nothing below them.

**One job system.** The exporter routes every worker through `bridge::Events`,
which repaints on send. The analyser holds an `mpsc` and calls
`request_repaint` itself. In one window, an analysis is a bridge job
(`spawn_blocking`, since the read is CPU-bound). The rule "nothing sends an
event by a route that does not wake the context" then has one mechanism, and
exports and analyses share one Activity list.

**State beside the exe, renamed.** `TelegramExporterData/` becomes
`TelegramToolsData/`. The first launch renames the old folder if the new one
is absent, then re-runs `ensure_data_dir`, so the ACL is re-applied with all
three of its guards. The session key moves with the folder; nobody signs in
again. `Exports/` keeps its name and place.

---

## 3. Phases

Each phase ends green on **every** check both repos run today, and changes
nothing it was not meant to change. Nothing in phases 0–2 alters any output
byte.

### Phase 0 — Freeze what "unchanged" means

1. Commit the pending work in each repo, in its own repo, separately.
2. Record the outputs the merge must not move:
   - `save.bat baseline` in the analyser: confirm it is green, then leave it as is.
   - the **digest** of UA KOLAB and KRGM, sha256'd into a file beside the
     baseline. This is the gap from §1.2.
   - `tga-db`'s model of `L:\9 telegram export`, every message field, dumped
     once through `rusqlite`. This is what the libsql port has to reproduce.
   - the exporter's four parity legs, `save.bat test`.
3. **Photograph every view of both windows**, so the redesign has a "before".
   The analyser's `TGA_SHOT` works offline. The exporter's `TGX_SHOT` signs in
   on launch when a session exists, which CLAUDE.md says needs your say-so. The
   fix is small: skip the startup sign-in when `TGX_SHOT` is set, and feed the
   chat list from a fixture. Then nobody's account is touched to take a picture.
4. Measure both release binaries' sections (`.text`/`.rdata`), so the merged
   size has something to be compared against.

### Phase 1 — One repository, one workspace, two binaries

1. **This repo becomes TelegramTools**, renamed by hand on GitHub. GitHub
   redirects a renamed repo, so existing clones keep working. The analyser comes in **with its
   history**:
   - in a scratch clone, delete `crates/tgx-ui`;
   - move `PLAN.md` and `REFACTOR.md` to `docs/analyser/`;
   - merge it in with `--allow-unrelated-histories`.

   `crates/tga-*` collides with nothing. The root files collide (`Cargo.toml`,
   `CLAUDE.md`, `save.bat`, `.gitignore`, `.gitattributes`), and each is
   resolved by hand in the merge commit.
2. One `Cargo.toml`, `preserve_order` on, and the analyser's comment about it
   rewritten to say the explicit sort is now permanently load-bearing.
3. Port `tga-db` onto `tgx-archive::Store::open_read`, then remove `rusqlite`.
4. Merge `.gitattributes`. The analyser's `-text` on goldens and assets, and
   `binary` on `*.ttf`, must survive.
5. Merge `save.bat`: one `test` runs both suites with both
   `TGX_REQUIRE_CORPUS` and `TGA_REQUIRE_CORPUS`. Add `baseline` and `bless`
   from the analyser, and one `build` that ships both exes to `dist\`.
6. CI: add `tga-*` to `cargo test --all` (it already is, by membership). The
   30 MB ceiling applies to each exe.
7. Merge both CLAUDE.md files into one, with a section per half and the shared
   rules (layering, repaint, corpus privacy, toolchain) stated once.

**Gate:** all four parity legs; the golden; `save.bat baseline` **byte
identical**; the digest hashes identical; the `tga-db` dump identical; both
corpus tests reading with `REQUIRE` on.

### Phase 2 — One executable, one window, the new frame

1. `tt-app` → `TelegramTools.exe`, **the only binary**. With no arguments it
   opens the window. With arguments it is the command line:
   - `analyse <folder> [--out …] [--digest] …` takes the analyser's existing
     argument set unchanged, so `save.bat report` and every documented
     invocation still work;
   - `login | chats | export | doctor | split | …` are `tgx`'s subcommands,
     moved over unchanged.

   `tgx-tg/src/bin/tgx.rs` (42 KB) becomes a `cli` module of `tt-app`, and
   the `tgx` binary is retired. The analyser already solved "a GUI-subsystem
   exe that also prints" when it went to one executable (v0.12), so reuse its
   console handling rather than inventing a second one. The `tgx` CLI's
   6.1 MB, window-free size is no longer something a build produces, and
   ROADMAP's size notes should say so.

   Every `cargo run -p tgx-tg --bin tgx -- …` in CLAUDE.md, `save.bat` and
   the docs becomes `cargo run -p tt-app -- …`. `save.bat wire` and the parity
   tooling that invoke `tgx` are found by grep and moved in the same commit.
2. `tga-app`'s non-UI half (`cli`, `refresh`, `job`, `state`) moves to
   `tga-run`, with its tests. `state.rs` was already "every rule the window
   applies, with no toolkit in it", so this is a move rather than a rewrite.
3. **The new navigation frame** (§4.2), with today's panel contents inside it
   as they are. The analyser's library becomes the Library view, and the
   Viewer moves under it.
4. `TelegramToolsData/` migration, window title, one new icon from
   `tools/make_icon.py`, and one `TGX_SHOT_VIEW` value per view.
5. Delete `TelegramExporter.exe`, `TelegramAnalyser.exe` and `tgx.exe` from
   `build`.

**Gate:** the phase 1 gate; size under ceiling; a shot of every view; the
interaction tests in `shell/tests.rs` ported, not dropped.

### Phase 3 — Things only one tool can do

These are the reasons to merge beyond tidiness, and most of them close items
already open in one repo or the other.

- **Library = the destination.** Its default root is `output_dir`, so there is
  no path to paste (it can still be pointed elsewhere). Each row is one export
  on disk, database or folder, showing messages, size, last sync and report
  state (none / out of date / current). Actions: **Read** (the Viewer),
  **Report** (analyse, or open if current) and **Show in folder**. This
  replaces both the analyser's list and the Viewer's database picker. Those
  were two listings of the same files.
- **Report after export**, a setting. When a chat finishes, its report is
  queued as the next Activity row. A database sync makes a report stale, and
  the analyser's staleness rule (older than the `.sqlite` or its `-wal`) is
  what decides that.
- **The language handshake** (analyser PLAN, "Still open"). The exporter
  writes the sidecar and the analyser reads it. Both halves are now one repo's
  to do.
- **Attachments inside a database** (analyser PLAN, "Still open"). Add
  `tga_docs::extract_bytes`, fed from `tgx-archive`'s blob read, which `tga-db`
  now goes through anyway.

### Phase 4 — The redesign, screen by screen

The design system first, in `tgx-ui` (§4.3). Then one screen per commit, each
with a before/after shot, in this order: frame → Chats → Activity → Library →
Settings → Sign-in → Viewer. Then the copy pass (§4.4) across all of them at
once, so the vocabulary changes in one place.

### Phase 5 — Close out

`README.md` for the one tool. `ROADMAP.md`, `AUDIT.md` and `DEFECTS.csv` stay
the exporter half's records, and `docs/analyser/` holds the analyser's. Archive
the TelegramAnalyser GitHub repo with a pointer, rather than deleting it.

---

## 4. The redesign

### 4.1 What reads as weird today

From the analyser's library shot (`%TEMP%\tga-lib.png`) and the exporter's
panel code. The exporter has not been photographed yet; that is phase 0.3.

1. **Tracked capitals carry data, not just labels.** Row captions
   (`1.2 GB · CHANGED YESTERDAY`), state marks (`NO REPORT`), every option
   label (`WRITE THE DIGEST`) and the whole status line, which also carries
   chat titles and raw RPC errors, are all set in letterspaced uppercase.
   Capitals are a strong mark. Used everywhere they mark nothing, and they are
   the slowest form of text to read.
2. **One bar, two kinds of control.** The exporter's top bar holds view tabs
   on the left and a numbered `01 Sign in · 02 Refresh · 03 Start export`
   sequence on the right. Its own comment records that the earlier two-bar
   version "read as two menus"; the one-bar version still asks the reader to
   tell them apart. Sign in is a state of the account, not a step you repeat.
   Refresh is a verb with no object.
3. **`[?]` everywhere.** Settings alone have about 25 help marks, some opening
   six-line paragraphs (`MODE_HELP`). What a setting does is hidden behind a
   hover, and the explanation of a choice is longer than the choice.
4. **Run options sit with selection verbs.** The analyser's footer puts
   `All · None · Needs a report` beside `Write the digest · Embed fonts`. One
   group changes which rows are picked; the other changes what the run writes.
5. **The same fact twice.** "40 exports, 1,486,313 messages" in the footer and
   "40 databases, none reported on yet" in the status bar, 30 points apart.
6. **A tick box with no tick.** A ticked box is a filled square. Without the
   row fill beside it, ticked and unticked differ only by fill.
7. **Dead space where the eye lands.** The analyser's footer is about 110 pt
   tall for one row of verbs and one line of text.
8. **Two vocabularies.** Classic/Database, Refresh, Count messages, Re-read,
   Sync every, Analyse, Report, Viewer: each is a fine word, but they were
   chosen in different months for different windows.

### 4.2 Information architecture

The tool does four things in order: **pick chats → export → keep → understand.**
The frame should say that once, and every screen should do one of those things.

```
┌──────────────┬──────────────────────────────────────────────────────────┐
│ TelegramTools│  Chats                                    [Export 12 ▸]  │
│              │  ┌ Filter chats ─────┐  Sort: Recent ▾   Show: All ▾      │
│ ▸ Chats   604│  ────────────────────────────────────────────────────────│
│   Activity  2│  ☑ UA KOLAB                       Forum · 4 topics  8,206│
│   Library  40│    Synced yesterday · report current                      │
│   Settings   │  ☐ Admini                                   Group  14,855│
│              │    Never exported                                         │
│              │  …                                                        │
│              │  ────────────────────────────────────────────────────────│
│              │  12 chats selected · 51,210 messages                      │
│              │  Select: All · None · Not yet exported   Saved: Weekly ▾  │
│──────────────│                                                           │
│ ● +381 6…  ▾ │                                                           │
│ Database mode│                                                           │
└──────────────┴──────────────────────────────────────────────────────────┘
  Exporting UA KOLAB — topic 2 of 4, 3,120 of 8,206 messages    ███░░ 38%
```

- **A left rail, not tabs.** Four destinations, each with its count.
  Vertical, so the labels can be words in sentence case and cannot be confused
  with actions. The **account** sits at the foot of the rail: signed in or not,
  as whom, and which mode is in force. Sign-in becomes a dialog opened from
  there, not step 01.
- **One primary action per screen, top right, named for what it does on that
  screen.** Chats: *Export 12*. Library: *Report 3*. Activity: *Stop*. It
  stays in one place and only its label changes, as Start → Add to queue does
  today. Refresh becomes a small reload control next to the list it reloads.
- **Activity** replaces Queue. It shows every run of either kind, export or
  report, one row each, with the log below. The transcript and the
  INCOMPLETE warning rules stay exactly as they are.
- **Library** shows what is on disk (§3, phase 3). The **Viewer** opens from a
  row and has a back link to the Library. It is a place you go into, not a
  sibling tab.
- **Settings** is grouped by question rather than by implementation. The
  current six sections become: *Where exports go*, *What an export contains*
  (mode, format, topics), *Media*, *Reports* (fonts, digest, handles only:
  options that today sit in the analyser's footer), *Appearance* and
  *Advanced* (re-read window, concurrency, schedule).
- **The status bar keeps one job:** what is running now, in sentence case, with
  progress on the right. When nothing runs it is empty. A finished run's
  summary goes in Activity, not here.

### 4.3 The design system, revised in `tgx-ui`

The identity stays: Geist, square corners, hairline rules, no shadow, **one
red, which means "this run" or "the primary action"**. That identity is not
the problem. The problems are which text roles are used where, and the
missing components.

| change | rule |
|---|---|
| **Capitals for labels only.** | `caps`/`eyebrow` only on a section label of three words or fewer. Data, states, options, status and errors are sentence case. |
| **A `SettingRow` component.** | Label, a one-line description always visible in `muted`, and the control on the right. Replaces most `[?]` marks. A longer explanation goes in an inline "More" disclosure, used only where a choice has consequences (mode, re-read, profile photos). |
| **A `Badge` component.** | Row state (`No report`, `Out of date`, `Failed`, `Running`) in a hairline box with sentence-case text. Accent for Running only, as today. |
| **A tick box with a tick.** | A glyph inside the box, not a fill. Ticked and unticked are told apart by shape, not by brightness. |
| **A `NavRail` component.** | Label, count, selected = ink plus a 2 pt bar on the leading edge. |
| **Spacing on the 4/8 grid, two row heights.** | 46 pt for a two-line row (title over caption) and 32 pt for a one-line row. Footers are as tall as their content. |
| **Numbers in tabular figures.** | Every count column right-aligned in GeistMono, as today. Keep that rule, and apply it everywhere a number moves. |

`tga-report`'s own CSS is **not** in scope. It is a file people keep, it has
a byte-for-byte baseline, and changing it is a separate decision with its own
`baseline record`.

### 4.4 The copy pass

One glossary, applied everywhere at once:

| today | becomes | why |
|---|---|---|
| Classic / Database | **Folder** / **Database** | Says what you get, not when it was invented. |
| Refresh (top bar) | ↻ beside the chat list, tooltip "Reload chats from Telegram" | A verb needs its object. |
| Count messages | **Count** in the list toolbar | Same object, closer to it. |
| Start export / Add to queue / Resume queue | **Export N** / **Add N to queue** / **Resume** | The number is the selection. |
| Queue | **Activity** | Holds reports too now. |
| Viewer | **Read** (the action), Library (the place) | You go to the Library and read a chat. |
| Analyse | **Make report** / **Open report** | Says what lands on disk. |
| Re-read the whole history | **Check the whole history** + "finds deletions and edits further back" | Says what the re-read is for. |
| NO REPORT / CHANGED | No report / Out of date | Sentence case, a state rather than an event. |

Error text keeps its specifics (the RPC name, the path), with one sentence of
what to do in front of it.

---

## 5. Risks

| # | risk | caught by |
|---|---|---|
| 1 | `preserve_order` reorders a `tga-*` output the explicit sort does not cover | baseline + digest hashes (phase 0.2) |
| 2 | libsql reads a field differently from rusqlite (NULL vs empty, integer widths) | the `tga-db` dump compare |
| 3 | libsql's read-only open still creates `-wal`/`-shm` beside a database the exporter is syncing | the analyser's existing no-side-files test, kept |
| 4 | `pdf-extract`, `zip` and `quick-xml` push the exe toward the ceiling | measured in phase 1; ceiling is 30 MB, combined estimate ~16 MB |
| 5 | Geist is embedded twice (`tgx-ui/fonts` and `tga-report/fonts`) | dedupe to one directory; `tga-report` keeps `include_bytes!` by path, still no UI dependency |
| 6 | the data-folder rename loses the session key, or the ACL | migration test on a temp dir; `ensure_data_dir` re-run after it |
| 7 | the merged `.gitignore` drops a line from one half, and chat history or the session key gets committed to a public repo | line-by-line check against both originals before the first push (§6) |
| 8 | the redesign breaks an interaction rule the code comments record (repaint, one writer, pinned view) | `shell/tests.rs` ported first; comments carried with the code they guard |

---

## 6. Decisions

Decided 2026-10-12:

1. **Repository:** the exporter repo is renamed TelegramTools, and the
   analyser is imported with its history. TelegramAnalyser on GitHub is
   archived with a pointer.
2. **Appearance:** light and dark in the window, and the report stays
   dark-only. The analyser's `theme.rs` copy, and its test against the report's
   palette, become a test that `tgx-ui`'s dark palette and the report's agree.
3. **Navigation:** a left rail, as in §4.2.
4. **The CLI:** everything goes into `TelegramTools.exe` and `tgx` is retired
   (phase 2, step 1).

5. **Visibility: public.** Both repos already are (checked 2026-10-12), so
   the merge publishes nothing new. The work happens in this repository, and
   the GitHub rename is done by hand on github.com. After that, `origin` is
   repointed with `git remote set-url`; the redirect covers the meantime.

Because the merged repo is public, the gitignore rules of both halves carry
over in full: `reference/`, `baseline/`, `Exports/`, `TelegramToolsData/`,
every `report*.html` and `*.stats.json`. Phase 1 checks the merged
`.gitignore` against both originals line by line before the first push.

---

## 7. Log

### Phase 0 — 2026-10-12

- Pending work committed in each repo: the exporter's "why a database run
  reads the whole history" log line, and the analyser's `--handles` (v0.15).
- **The analyser's baseline could not run.** Its two folder corpora were not
  mounted (`N:` was a different disk that day, and `J:\temp pureraw` held no
  KRGM). So the recording was taken with what was on hand:
  `tools/merge_check.ps1` writes the report and stats dump for `reference\`
  (the exporter's cut of the same UA KOLAB export) and three databases on
  `L:`, the digest, and `model_hash`, a hash of every field of the database
  model, per topic: 49 lines over 221,680 messages.
- **The exporter's screenshots no longer sign in.** `TGX_SHOT` used to connect
  with the stored session. It now shows an invented account. "Before" shots of
  every view of both windows are in `target\shots\before\`.
- Sizes: `TelegramAnalyser.exe` 10.6 MB; `TelegramExporter.exe` 12.4 MB
  (`dist\`, as last built).

### Phase 1 — 2026-10-12

- The analyser's history is merged in (merge commit `17fdbda`). Its copy of
  `tgx-ui` was byte-identical to the exporter's and is gone.
- **The merge checkout made every analyser file CRLF**, golden and report
  assets included, because this repo had no `.gitattributes`. The analyser's
  targeted rules came over, but not its repo-wide `eol=lf`, and the seven
  byte-compared files were checked out again.
- **`preserve_order` changed nothing.** With it on, all 49 manifest lines
  matched the recording. One false alarm on the way: the input folder had been
  copied inside each run's own output directory, and the stats dump records the
  export's path, so the folder legs differed by that path alone. The input now
  goes to one fixed scratch path.
- **`rusqlite` and `libsql` coexist in one workspace.** They use different
  native link names, so cargo resolves both, and only a binary linking the two
  would fail. That binary is phase 2's `TelegramTools.exe`. The port was still
  done here, as planned.
- **`tga-db` reads through `tgx_archive::Store::open_read`.** Four read
  methods were added to the store: `chat_summaries`, `chat_titles`,
  `participants_of`, `edit_counts`. All 49 manifest lines matched again. The
  corpus test reads 63 databases (1,608,533 messages).
- **The "no `-wal`/`-shm`" test was never true of a real export.** Its old
  fixture was hand-built in rollback mode. Built by the exporter's `Store`
  instead, it is WAL, and SQLite creates the pair for any reader, rusqlite
  included: every database on `L:` already had them. The test now asserts that
  the database's bytes are unchanged and its log stays empty.
- **The `tga-db` corpus test crashed twice with an access violation**
  (`0xc0000005`), in about a dozen runs, both times in a binary built by a
  multi-crate `cargo test`. Four parallel runs of the single-crate build did
  not crash. The test opens stores on five threads at once.
  Reading libsql 0.9.30 found one use-after-free on that path: a connection's
  SQLite handle is closed twice on drop (`LibsqlConnection::drop` and then
  its field's own `Drop` both call `disconnect()`). `tgx_archive::Store` now
  holds one never-run statement so the handle is closed exactly once (see
  `_close_once`). A stress test of many stores on eight threads did not crash
  either before or after the fix, so **the double close is not proven to be
  the crash.** The exporter's own window opens read-only stores on worker
  threads while an export writes, so it was exposed to the same double close.
- `tga-read`'s corpus test falls back to `reference\`, so it runs without the
  drive.
- `layering.rs` holds the analyser's rules: no `tga-*` reaches Telegram, no
  `tgx-*` reaches `tga-*`, the report depends on neither the reader nor the
  metrics, the notes depend on neither the reader nor the documents, and only
  `tga-app` reaches `tga-docs` and `tga-db`.
- One `save.bat` and one `CLAUDE.md`. CI builds, size-checks and uploads both
  exes.

### Phase 2 — 2026-10-12

- **One binary, `TelegramTools.exe`** (`crates/tt-app`, was `tgx-app`). No
  arguments open the window; `analyse`, `analyze` or `report` hands the rest of
  argv to the analyser's command line unchanged; anything else is the exporter's
  (`tgx.rs` moved in as `tt-app/src/cli.rs`, and builds its own tokio runtime
  instead of owning `main`). The analyser's `AttachConsole` came with it. The
  analyser's command runs before the log opens, so a report written from a
  terminal does not rotate the window's last `tgx.log` away.
- `tga-run` is the analyser's front end without a window: `cli`, `refresh`,
  `job`, `state`, `open`. `job` used to hold a clone of the egui context to
  repaint with; it holds a `Wake` callback now. `state` called
  `tgx_ui::selection_label`, which would have put a toolkit back in, so the
  footer's wording is written out there. `layering.rs` checks that `tga-run`
  has no window in it, that only it reaches `tga-docs`/`tga-db`, and that
  `tt-app` names only `tgx-ui`, `tgx-tg` and `tga-run`.
- **A report is renamed into place.** The window does not wait for the
  analyser's thread on quit (the analyser's own window did not either), and a
  report is judged current by its modification time, so one cut short would
  have read as up to date for good. `refresh::one` writes `report.html.part`
  and renames it. Same bytes; the manifest still matches.
- **The frame**: a left rail lists the views with their counts, the account at
  its foot (a Sign in button when there is none); the top bar names the view
  and carries that view's actions. The Library view is the analyser's window,
  opened on the exporter's destination the first time it is shown, and its own
  status line sits at its foot. **Not done as planned:** the Viewer is still
  its own rail entry rather than under the Library. Folding it in means the
  Library's rows opening a database in the Viewer, which is phase 3's merged
  Library, not a move.
- **`TelegramExporterData` → `TelegramToolsData`.** `config::resolve_data_dir`
  renames the old folder when it finds it alone, and keeps using the old one if
  the rename fails. The workspace root's own folder was renamed by the first
  test run, session included.
- **Gates.** `merge_check` through `TelegramTools.exe analyse`: all 49 lines
  identical to the phase 0 recording. Size 18.3 MB (was 12.4 + 10.6). The
  interaction tests in `shell/tests.rs` came over whole; one was rewritten for
  five views and one added for the rail's account. Shots of every view are in
  `target\shots\after\`. The `tga-docs` corpus still cannot run (drive not
  mounted).
- **Seen in the shots, left for phase 4:** the bar still offers `01 Sign in`
  when the rail already says who is signed in, and the Library shows its own
  status line above the window's, which still talks about the chat list.

### Phase 3 — 2026-10-13

- **The Library is the one listing of what is on disk.** It follows the
  destination when Settings changes it, unless it was pointed elsewhere by
  hand. A destination lists its databases **and its Classic folders, one row
  per chat**: `refresh::library` used to read a folder holding no database as
  one export, which on a Classic destination meant every chat as one. A folder
  is one export when it has `result.json` or `export_results.html` at its
  root, which Desktop writes at every export's root and the exporter writes
  above a forum's topics.
- Each row has **Read** (a database: opens it in the Viewer), **Make report**
  or **Open report** (whichever is true — current or not), and **Show in
  folder**. Make report is that row alone; the ticks are left as they were.
- **The Viewer left the rail** and lost its database picker. It is entered by
  Read and has a `← Library` link back; the rail lights Library while it shows.
- **Report after export**, a setting under a new Reports section. Each chat is
  handed to the Library as it finishes — its database if it has one, else its
  folder — and the reports are written while the next chat downloads.
  `ExportResult` now carries the database path, because a title clash puts the
  chat id in the file's name and the window should not re-derive it.
- **Attachments inside a database**: `tga_docs::read_bytes`/`extract_bytes`
  beside the path versions, which now read the file and call them. The digest
  reads a document from disk when it is there and from `blobs` otherwise
  (`tga_run::cli::Stored`), so a folder export's digest is unchanged.
- **The language handshake was not built.** Neither premise holds: the exporter
  never asks which language an export is in, and the report has no string
  lookup. A language setting with nothing to switch to would be a control that
  does nothing. It needs a decision on which languages and who writes the
  strings; recorded in `docs/analyser/PLAN.md`.
- **Gates.** `merge_check` 49/49 identical (the three `L:` databases hold no
  stored documents, so their digests did not move). Size 18.3 MB.
- New icon (`tools/make_icon.py`): the exporter's upper rule and the analyser's
  baseline and bars, the red bar reaching from one to the other.
- `save.bat build` stops and removes `TelegramExporter.exe` and
  `TelegramAnalyser.exe` from `dist\`; `baseline`, `report` and `run` go
  through `tt-app`. CI builds, size-checks and uploads the one exe.

### Phase 4 — 2026-10-13

- **One commit, not one per screen.** The screens share the frame and the new
  components, and each intermediate state would have been a window half in
  one vocabulary and half in the other. Before: `target\shots\phase3\`;
  after: `target\shots\phase4\` (chats, activity as `queue`, library on
  `reference\`, settings at the default size and at the 900x620 floor).
- **`tgx-ui`:** `badge` with three `Tone`s (quiet, ink, accent — accent only
  for this run); `setting_row` (name, one muted line, control on the right,
  laid out control-first so the line wraps instead of pushing it off);
  `nav_item` (sentence case, count in the mono, a 2pt bar when selected,
  32pt, the whole width the hit target); the tick box draws a tick instead of
  a fill. A test for each. The module docs state the capitals rule.
- **Frame.** The rail uses `nav_item`. The numbered `01/02/03` bar is gone:
  one primary per view, top right — Chats *Export N chats* / *Add N to queue*
  / *Resume* (or *Sign in* while nobody is), Activity *Stop*, Library *Make N
  reports*. Sign in otherwise lives at the rail's foot. The view's name is a
  sentence-case heading. The status bar is no longer uppercased, no longer
  says "8 chats" after a reload, and carries the analyser's line (and its
  bar) while the Library shows or a report run goes — the Library's own
  status line is gone.
- **Chats.** Reload and Count sit in the filter row; Update databases stays
  in the foot with the selection verbs. On-disk and converted-group marks
  are badges; captions, verbs and the selection total are sentence case.
- **Activity** (was Queue). A Reports section above the exports lists this
  session's report runs with badges, a bar and the analyser's status. Stop
  stops both. Export states are badges, a failure is its own error in ink.
- **Library.** Badges for the marks, sentence-case row actions; the foot is
  selection only. *Embed fonts*, *Write the digest* and *Handles only* are
  persisted settings now (`report_fonts`, `report_digest`,
  `report_handles`), handed to the Library each frame.
- **Settings.** Regrouped into Where exports go / What an export contains /
  Media / Reports / Appearance / Advanced, each row a `setting_row`. Three
  `[?]` remain: the format, Check the whole history, Profile pictures.
- **Copy.** Classic→Folder in every visible string (the setting key is still
  `export_db`); Re-read the whole history→Check the whole history; Queue→
  Activity; Viewer→Read; Refresh→Reload; Changed→Out of date; Analysing→
  Making report; Export as Classic→Export as folder. Sign-in errors get one
  sentence of what to do before the error itself.
- **Not done:** the light appearance was not photographed; `tga-report`'s CSS
  was out of scope, as planned.

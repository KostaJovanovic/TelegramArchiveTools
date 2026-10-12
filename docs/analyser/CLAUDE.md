# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Point `TelegramAnalyser.exe` at a finished Telegram export folder and it writes
one self-contained `report.html` beside it. Run it with no arguments and the
same program opens a window instead.

**One executable.** It shipped as two — the window, and `tga.exe` for the
command line — until the only difference between them turned out to be how they
were launched, which argv already records.

**Read `PLAN.md` first.** It is the record of why each figure is computed the
way it is, and of the decisions that would otherwise be re-litigated every time
somebody reads the code. `REFACTOR.md` is the log of the rewrite that produced
the current shape — worth reading once, for what it found.

## Commands

`save.bat` is the entry point — bare for a menu, or with an action. It is
TelegramExporter's `save.bat` with this repo's checks in it; keep the two in
the same shape.

```powershell
save.bat test               # fmt + clippy + every suite
save.bat save               # fmt + clippy + suite + baseline, commit, push
save.bat commit             # the same, no push
save.bat --force            # save, then force-with-lease push without asking
save.bat release            # save, then build
save.bat build              # release build into dist\
save.bat run                # the window
save.bat report <folder>    # write report.html beside an export
save.bat baseline           # both archives against the last recording
save.bat baseline record    # re-record it
save.bat bless              # re-record the committed golden
save.bat clean              # report target\ size, then empty it
```

Underneath it is plain cargo. Single legs:

```powershell
cargo test -p tga-report --test golden     # the golden; no corpus needed
cargo test -p tga-read   --test corpus     # the reader against a real export
cargo test -p tga-db     --test corpus     # the exporter's own telegram.sqlite
cargo test -p tga-docs   --test corpus     # every attachment, read for real
cargo test --all -- --nocapture            # --nocapture matters; see below

cargo run -p tga-app --bin TelegramAnalyser -- <folder or telegram.sqlite>
        [--out P] [--chat ID|TITLE] [--digest] [--no-fonts] [--stats P]
        [--notes P] [--stamp T]
cargo run -p tga-app --bin TelegramAnalyser -- <exporter output folder> --refresh [--force]
cargo run -p tga-app --bin TelegramAnalyser -- --from-stats <s.json> --out <p>
```

- **`TGA_REQUIRE_CORPUS=1`** (which `save.bat` sets) turns a missing corpus into
  a failure instead of a skip. A plain `cargo test --all` still skips, which is
  what keeps a fresh clone working. The skips are `eprintln`s, so always pass
  `-- --nocapture` — libtest discards them otherwise and a suite that compared
  nothing reports as coverage.
- **`TGA_BLESS=1 cargo test -p tga-report --test golden`** re-records the
  golden. Read the diff before committing it.
- `TGA_EXPORT` overrides the reader test's corpus path; `TGA_DOCS_EXPORT` does
  the same for the document test, which needs an export with attachments
  actually downloaded; `TGA_DATABASE` for the database test.
- **The document corpus leg is the slow one** — a minute in a debug build,
  because it parses 238 PDFs. That is the cost of the only test that proves
  `pdf-extract` survives real input, so it is not worth trimming; it is worth
  knowing about before assuming the suite has hung.

## Layering, and the two rules that pay for themselves

```
tga-read      export folder -> model. Both layouts (ours: result.json per topic
              at the root; Desktop: chats/chat_<id>/result.json). No UI, no network.
tga-db        telegram.sqlite -> the same model. The exporter's database output
              is a different container, not a different format: it hands the
              stored JSON to tga-read rather than parsing it again. Depends on
              tga-read; only tga-app depends on it.
tga-docs      a .txt/.docx/.pdf attachment -> its text and its real date.
              Nothing depends on it but tga-app, on purpose: it carries a zip
              reader, an XML parser and a PDF parser, and none of those belong
              anywhere near tga-report.
tga-notes     the hand-written annotation layer + the model-facing digest.
              MUST NOT depend on tga-read or tga-docs.
tga-stats     the shape of every figure, and the dump's format. Data only.
tga-metrics   every figure, one pass. No I/O. Fills in a tga_stats::Stats.
tga-report    the ramp, the SVG marks, the one HTML file. Reads a Stats.
              MUST NOT depend on tga-read or tga-metrics.
tga-app       the only binary, TelegramAnalyser.exe. cli.rs is the argument
              half; app.rs is the window, egui on glow; state.rs holds every
              rule the window applies, with no toolkit in it, so they stay
              testable. refresh.rs is the library both halves share: every
              export under a folder, whether its report is stale, and the one
              function that analyses one.
tgx-ui        TelegramExporter's design system, **copied verbatim** (see the
              workspace Cargo.toml). Edit it in the exporter and copy it back.
```

**The window is a library.** Point it at a folder and every export in it is
a row — the exporter's per-chat databases, or one folder export — with what
needs a report already ticked; Analyse runs the ticks in turn. It is laid out
with the exporter's own pieces so the two windows are one product, and
`tools\shot.ps1 -Folder <path>` photographs it (`TGA_SHOT`; reading the
screen from outside comes back black under GL). The icon is drawn by
`tools\make_icon.py` and embedded by `crates/tga-app/build.rs`; regenerate it
there, never by editing the `.ico`.

`tga-report` renders from a `tga_stats::Stats`, which is exactly what `--stats`
dumps. That is what lets a recorded dump replay through the writer with no
export on disk — the golden test and `--from-stats` both rest on it. **The
shape lives in `tga-stats` rather than in either side**, so the writer can read
it without depending on the reader or the metrics, and the compiler checks every
field name. `tga-notes` inherits the rule at one remove because it carries the
`Event` type `tga-report` renders; the `Export -> digest::Row` mapping therefore
lives in the *caller* (`tga-app/src/cli.rs`). Reading an attachment is the same
rule one step further out: `tga_notes::Attachment` is all strings, and
`tga-app/src/cli.rs` is what calls `tga-docs` and formats the dates into them.

Every field of `Stats` has a `Default` and the struct is `#[serde(default)]`, so
a dump with a branch missing renders an empty section rather than failing to
load. `dynamics` is an `Option` on purpose: absent is "recorded before this
existed, draw no section", which is a different fact from an archive with
nothing in it.

## Invariants that change numbers or break the harness

- **Time is two clocks.** Anything with a calendar or clock face reads
  `Msg::when` (naive local wall clock); anything measuring a *duration* reads
  `Msg::unix`, which stays monotonic across DST. Decided once in `tga-read`.
- **A deleted message still counts.** A `telegram.sqlite` keeps a message
  Telegram no longer returns, marked with the date it went missing, and
  `tga-db` does *not* filter on `deleted_seen`. Keeping what was deleted is the
  whole reason that format exists; leaving it out of the figures would make the
  archive agree with Telegram, which is the one thing it is meant not to do.
  The count is reported separately so nobody has to reconcile the report with a
  live chat and lose.
- **A document has its own date, and it is not the message's.** An attachment
  is dated by `tga-docs` from its filename and its text, never from the
  filesystem — every file in an export carries the mtime of the moment the
  exporter wrote it, and all 44,000 in the KRGM corpus say `2026-08-27`. The
  post date is the last resort and the digest says so: `doc.src` is one of
  `filename`, `content`, `filename+posted` or `posted`, and dropping it would
  leave a reader unable to tell a written date from a guessed one. This matters
  because people post archives — five KRGM zapisnici spanning December 2024 to
  February 2025 were all uploaded on one afternoon in September.
- **A sender is a typed peer key, never a display name.** Grouping by name
  splits one person across their renames and merges two people who share one.
- **A forum topic is a thread**, so every top-level message in one is marked as
  a reply to the message that opened it. Read literally, response-time medians
  stretch to days.
- **The stats dump is sorted by `tga_stats::write`, not by the map type.**
  `serde_json`'s object is a `BTreeMap` and sorts itself — until something turns
  on its `preserve_order` feature, and cargo unifies features across everything
  built in one invocation. The window's old toolkit turned it on, so building
  the command-line binary alone and building the workspace produced differently
  ordered dumps from the same numbers. The sort is explicit now; do not remove it on the
  grounds that BTreeMap already does it, and do not assume the current
  dependency set is the last one that will reach for that feature.
- **`assets/report.css` and `assets/report.js` are copied into the report
  verbatim**, comments included. Editing either changes the file's bytes and
  fails `save.bat baseline`, and `.gitattributes` pins their line endings for
  the same reason it pins the golden's.
- **`Count` serialises as `["label", 41]`, not as an object.** Nine branches use
  it. The `from`/`into` pair on the struct is what keeps the file's shape while
  the code reads `.label` and `.n`; changing it invalidates every recorded dump.
- **Nothing in a notes file is trusted.** Every string is escaped into the HTML
  and a malformed entry is dropped, never raised — which is why `tga_notes::load`
  returns no `Result`.
- **One file, no network.** Fonts base64'd into the stylesheet, charts inline
  SVG, every view rendered server-side; JS only filters and pans what is already
  drawn. Target: under 1 MB for a 350k-message archive.

## Verification

**`save.bat baseline` is the load-bearing check.** It runs the program over both
real archives and compares the whole report and the whole stats dump, byte for
byte, against a recording. A difference is a mistake rather than a judgement
call unless the change was meant to alter the output — and then it is
`save.bat baseline record`, after reading the diff. Three legs: `ua-kolab`,
`krgm`, and `krgm-notes`, which passes the 42 hand-written notes so the whole
annotation layer's markup is covered too.

Four real corpora, all on removable drives:

| corpus | messages | topics | what it is for |
|---|---:|---:|---|
| `N:\telegram export\UA KOLAB TELEGRAM` | 6,643 | 4 | baseline |
| `J:\temp pureraw\KRGM*` | 333,582 | 10 | baseline |
| `N:\telegram_export\KROVNA RADNA GRUPA ZA MEDIJE*` | 450,817 | 33 | documents: 150 docx, 238 pdf, 9 txt |
| `L:\9 telegram export\UA KOLAB\UA KOLAB.sqlite` | 8,206 | 7 | the database format |

The fourth is the exporter's database output, not a folder: UA KOLAB again,
re-exported with only the Database format on, so **there is no `result.json`
anywhere in it**. That is exactly why it is worth keeping — an export that the
folder reader cannot see at all. It used to be one flat `telegram.sqlite`
holding two chats; `tgx split` turned it into one database per chat,
`<title>\<title>.sqlite`, and `L:\9 telegram export` now holds 40 of them
(36 GB, 1.49M messages). The corpus test reads every one, because that folder
is what `--refresh` and the window's Refresh walk: a report is redone when it
is older than its database (or its `-wal`, the exporter syncs in WAL mode) or
its notes file.

The third is the same group as `KRGM` re-exported later, and it is the only one
with the media downloaded — which is what makes it the document test's corpus.
**The `.txt` files at each topic's root are not attachments.** They are
`missing_media.txt`, the exporter's log of what it failed to fetch, and there
are 30 of them against 5 real text files; anything walking the tree for
documents must look in `<topic>/files/` only. The same export also *references*
133 `.docx` it never downloaded, so a document that reads as empty is usually a
missing file rather than a broken parser — check the disk before the code.

Re-record with `save.bat baseline record` **only** for a change that is meant to
alter the output, and read the diff first. The digest is not in the baseline:
it is written beside the export, not into `baseline/`, so a change to
`tga-docs` or to `digest.jsonl`'s shape passes all three legs untouched. That
is correct — the report does not read a single attachment — but it does mean
the document work has only its own corpus test behind it.

`baseline/`, `reference/`, `report*.html` and `*.stats.json` are gitignored —
they are verbatim chat history from real people. (`reference/` holds output from
the deleted Python harness; nothing reads it any more and it can be removed.)
The committed fixtures (`tga-report/tests/fixture.stats.json`,
`tests/synthetic/`) are hand-written and name nobody, which is why they can be
committed and never skip. They catch drift on a fresh clone with no drives; they
do not claim correctness.

`.gitattributes` forces `-text` on every golden and recorded file (a CRLF
checkout would fail every byte-for-byte compare), `eol=crlf` on `*.bat` (cmd's
`goto` cannot find a label in an LF-only batch file), and `binary` on `*.ttf`.

## Conventions

- Comments here record *what went wrong* and *what was ruled out*, not what the
  code does. Preserve them through edits; a paraphrase loses the defensive
  detail that was added after something broke. New non-obvious decisions get the
  same treatment.
- Dark only. One palette in `tga-app/src/theme.rs`, one `html.dark` block in the
  report, no switch. `tga_report::palette::tests` re-derives all four ordinal
  ramp checks from the hex values, and `tga_app::theme::tests` asserts the
  window's copy has not drifted from the report's.
- The window is `eframe`/`egui` on the **glow** backend, deliberately: it is a
  list and a row of buttons, and wgpu drags in a shader compiler for that.
  Everything the design needs and egui does not default to — square corners,
  hairline strokes, no shadow, Geist — comes from `tgx_ui::install`.
- Toolchain is pinned: Rust 1.97.0, MSVC target.
- `origin` is `github.com/KostaJovanovic/TelegramAnalyser`, which is **public**
  — another reason `baseline/`, `reference/` and every real report stay
  gitignored. `save.bat` asks before every push and only ever forces with
  `--force-with-lease --force-if-includes`.

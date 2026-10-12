# REFACTOR.md

Cutting this program loose from the Python original it was built to copy, and
cleaning up what that copying left behind.

**Done.** This is the record of what was agreed, what was done, and what it
found. `PLAN.md` describes the program that came out of it; `CLAUDE.md` is the
working guide.

Written in plain words on purpose.

---

## What we agreed

1. **Before deleting anything, save today's output.** Run the current program on
   both archives, keep the results, and check every later step against them.
2. **Real Rust types for the statistics**, and keep the ability to save them to
   a file and rebuild a report from that file later.
3. **The window drops GPUI and uses egui instead** — plain buttons and text,
   close to the old look.
4. **The statistics and report code get cleaned up too.**

## What was wrong

Five things, in the order they cost:

1. **The statistics were an untyped JSON blob.** `analyse()` returned a
   `serde_json::Value`, so every read was a string lookup with a fallback —
   about 215 of them. Rename a field and nothing failed to compile; the report
   silently showed zero, and a zero in a report looks like a fact.
2. **The report had two versions in one file.** A `classic` flag switched
   between the real report and a character-for-character copy of the Python's
   HTML, with about twenty `if classic` branches scattered through the rendering
   code, a set of light-mode colours only the copy used, and a bug reproduced on
   purpose so the comparison stayed clean.
3. **Three files were too big** — 1,927 lines, 1,648 and 1,040, and about 500 of
   the last were CSS and JavaScript pasted inside Rust string literals.
4. **Half the tests could not run** anywhere but on this machine, with both
   archives plugged in and the Python code in a sibling folder.
5. **The window carried 13.6 MB for one text field.**

## What was done

| step | | check |
|---|---|---|
| 0 | commit the staged `dynamics` work | v0.04 |
| 1 | record today's output as the baseline | v0.05 |
| 2 | delete the Python connection | v0.06 |
| 3 | real types for the statistics | v0.07 |
| 4 | split the oversized files, move CSS/JS to real files | v0.08 |
| 5 | the window, in egui | v0.09 |
| 6 | rewrite the documentation | v0.10 |

**`save.bat baseline` is what replaced the oracle**, and it asks the better
question. The Python harness could only ever answer "does this still match the
program it was ported from", which stops meaning anything the moment the two are
supposed to differ. The baseline asks "did anything I just changed alter one
byte of the report?", on both real archives, on three legs — the third passes
the 42 hand-written notes so the annotation layer's markup is covered too.

It was confirmed to go red before it was trusted: one appended byte fails the
leg, and re-recording clears it. **Steps 2, 3 and 5 changed nothing.** Step 4
changed 18 lines on a 542 KB report, and every one of them was intended.

## What it cost, and what it found

**Step 2** deleted 2,044 lines and added 308. The `classic` flag was a genuinely
good idea for as long as it lasted — it kept two million characters checked
against a working implementation through a phase that was supposed to end that
check. What it cost was a second document living inside the first, with every
feature written once and then fenced off, and a frozen path nobody was allowed
to tidy. That price only made sense while somebody was collecting on the diff.

**Step 3** was the largest, and it found a live bug that had nothing to do with
the refactor. A `tga-stats` test passed under `cargo test -p tga-stats` and
failed under `cargo test --all`: `serde_json`'s object is a `BTreeMap` and sorts
itself, but its `preserve_order` feature swaps in an `IndexMap`, and cargo
unifies features across everything built in one invocation. `gpui` turned that
feature on. So `cargo build -p tga-cli` and `cargo build` — which is what
`save.bat build` runs — produced differently ordered stats dumps from the same
numbers, and only one of them matched a recording. `tga_stats::write` now sorts
every object explicitly, at every level and inside arrays.

**Step 4**'s one deliberate change was the `details[open]` marker, which had
shipped a C1 control character followed by an ASCII `2` — what a minus becomes
after a bad encoding round trip, drawn by every browser as a stray "2". It was
reproduced on purpose while a byte diff against another program's output was
worth keeping. It is a minus sign now.

**Step 5** came in at 4.5 MB against 15.2, and 13.6 of the 15.2 were one text
field: `gpui-component` was in the tree for the path box and nothing else. Both
pre-1.0 pins went with it. The window's rules moved into a `state.rs` with no
toolkit in it, and thirteen tests became twenty — two of the new ones cover
things the old window had no way to check without opening one.

## Where it ended up

```
                    before          after
largest file        1,927 lines     835 (tga-stats, almost all declarations)
report crate        3 files         19, one per section and per kind of mark
stats interface     JSON blob       ~40 named structs the compiler checks
window              15.2 MB         4.5 MB
tests               ~150            ~180, and all of them run on a fresh clone
Python needed       for half        never
```

The report a reader opens is the same report, except for the minus sign.

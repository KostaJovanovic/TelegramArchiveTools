//! Everything the window decides, with no window in it.
//!
//! **This is the half worth testing**, and it is separated for that reason
//! rather than for tidiness: which rows a filter shows, what Analyse would do,
//! how full the bar is and what the status line says on a finished run are all
//! rules that read fine and are wrong on one branch. None of them needs a
//! toolkit to check, and none of them changed when the toolkit did.
//!
//! **One writer per fact.** The status line has one setter and a row's outcome
//! is written by [`State::apply`] alone, because the second writer always wins
//! a race nobody knew they had.
//!
//! **The window is a library, not a form.** It used to be one path, two tick
//! boxes and a button, which fitted an export that was a folder written once.
//! The exporter now syncs one database per chat into one folder — forty of
//! them on the reference drive — and a form over that is a form filled in
//! forty times. So the folder is chosen once and every export in it is a row,
//! with what is stale already ticked.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Local};

use crate::cli::thousands;
use crate::job::{Counted, Progress};
use crate::refresh::{Source, Target};

/// One export in the library.
#[derive(Debug, Clone)]
pub struct Entry {
    pub target: Target,
    /// The chat's title once the file has been opened; the file's own name
    /// until then.
    pub title: String,
    /// `None` until counted, and for a folder export, which is never counted.
    /// **Not zero**: a blank and a zero are different facts.
    pub messages: Option<usize>,
    /// Chats in the file. More than one only for a flat `telegram.sqlite`.
    pub chats: usize,
    /// The chat a database holds — its largest, as for `title` — once the
    /// file has been opened. What the window's Read needs to open the Viewer
    /// on it. Always `None` for a folder export.
    pub chat_id: Option<i64>,
    pub outcome: Outcome,
}

/// What this session has done to a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Idle,
    Queued,
    Running,
    Done,
    Failed(String),
}

/// The one word a row carries on its right, in the order they outrank each
/// other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Running,
    Queued,
    Failed,
    NoReport,
    Changed,
    Current,
}

impl Mark {
    pub fn label(self) -> &'static str {
        match self {
            Mark::Running => "Making report",
            Mark::Queued => "Queued",
            Mark::Failed => "Failed",
            Mark::NoReport => "No report",
            Mark::Changed => "Out of date",
            Mark::Current => "Up to date",
        }
    }
}

impl Entry {
    pub fn mark(&self) -> Mark {
        match &self.outcome {
            Outcome::Running => Mark::Running,
            Outcome::Queued => Mark::Queued,
            Outcome::Failed(_) => Mark::Failed,
            Outcome::Idle | Outcome::Done => match (self.target.reported, self.target.stale) {
                (None, _) => Mark::NoReport,
                (Some(_), true) => Mark::Changed,
                (Some(_), false) => Mark::Current,
            },
        }
    }

    /// The report, if there is one to open.
    pub fn report(&self) -> Option<&Path> {
        self.target
            .reported
            .is_some()
            .then_some(self.target.out.as_path())
    }

    /// The caption under the title: what is on disk, when it last moved, and
    /// when the report was written — or why the last attempt failed.
    pub fn caption(&self, now: SystemTime) -> String {
        if let Outcome::Failed(why) = &self.outcome {
            return why.clone();
        }
        let mut parts = vec![match &self.target.source {
            Source::Database(_) => human_bytes(self.target.bytes),
            Source::Folder(_) => "Folder export".to_string(),
        }];
        if self.chats > 1 {
            // Read as its largest. Said, because the others are in the file and
            // in no report.
            parts.push(format!("{} chats, largest read", self.chats));
        }
        if let Some(at) = self.target.changed {
            parts.push(format!("changed {}", human_when(at, now)));
        }
        if let Some(at) = self.target.reported {
            parts.push(format!("report {}", human_when(at, now)));
        }
        parts.join("  ·  ")
    }
}

/// Which rows the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    All,
    Behind,
    Current,
}

impl Show {
    pub const ALL: [Show; 3] = [Show::All, Show::Behind, Show::Current];

    pub fn label(self) -> &'static str {
        match self {
            Show::All => "Every export",
            Show::Behind => "Needs a report",
            Show::Current => "Up to date",
        }
    }

    fn admits(self, entry: &Entry) -> bool {
        match self {
            Show::All => true,
            Show::Behind => entry.target.stale,
            Show::Current => !entry.target.stale,
        }
    }
}

/// The order the list is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Behind,
    Name,
    Largest,
    Recent,
}

impl Sort {
    pub const ALL: [Sort; 4] = [Sort::Behind, Sort::Name, Sort::Largest, Sort::Recent];

    pub fn label(self) -> &'static str {
        match self {
            Sort::Behind => "Needs a report first",
            Sort::Name => "Name",
            Sort::Largest => "Most messages",
            Sort::Recent => "Recently changed",
        }
    }
}

pub struct State {
    /// Exactly what is in the path field. The folder is read back out of it
    /// rather than kept in a second string: two copies of the same path is how
    /// a status line ends up describing a folder other than the one on screen.
    pub path: String,
    /// The folder the library was last read from.
    pub root: Option<PathBuf>,
    pub entries: Vec<Entry>,
    /// Held by path, so ticks survive filtering, sorting and a rescan.
    pub selected: HashSet<PathBuf>,
    pub filter: String,
    pub show: Show,
    pub sort: Sort,
    pub write_digest: bool,
    pub embed_fonts: bool,
    /// `--handles`. Off by default, so the window and a bare command line
    /// write the same report.
    pub handles_only: bool,

    pub status: String,
    pub running: bool,
    /// Pressed Stop; the rows in hand are finishing.
    pub stopping: bool,
    /// The rows being read now and what each last said, in the order taken.
    /// A run reads several at once (`job::MAX_WORKERS`).
    in_hand: Vec<(PathBuf, String)>,
    /// Exports finished in this run, of [`State::of`].
    pub done: usize,
    pub of: usize,
    /// Bumped whenever the folder changes, so a count from the last one is
    /// recognised and dropped.
    pub generation: u64,
    /// What this run wrote and what it could not.
    written: usize,
    failed: Vec<String>,
    /// Exports that appeared while a run held the rows, found by the rescan
    /// that ends it and not yet handed to `job::count`. See
    /// [`State::take_uncounted`].
    uncounted: Vec<Source>,
}

impl Default for State {
    fn default() -> Self {
        State {
            path: String::new(),
            root: None,
            entries: Vec::new(),
            selected: HashSet::new(),
            filter: String::new(),
            show: Show::All,
            sort: Sort::Behind,
            write_digest: true,
            embed_fonts: true,
            handles_only: false,
            status: "Choose an export folder.".into(),
            running: false,
            stopping: false,
            in_hand: Vec::new(),
            done: 0,
            of: 0,
            generation: 0,
            written: 0,
            failed: Vec::new(),
            uncounted: Vec::new(),
        }
    }
}

impl State {
    /// What is in the field, with the quotes Explorer's "Copy as path" adds.
    pub fn folder(&self) -> Option<PathBuf> {
        let text = self.path.trim().trim_matches('"');
        if text.is_empty() {
            None
        } else {
            Some(PathBuf::from(text))
        }
    }

    pub fn set_path(&mut self, folder: &Path) -> Vec<Source> {
        self.path = folder.to_string_lossy().into_owned();
        self.rescan()
    }

    /// Read the folder in the field into rows, and say what is there.
    ///
    /// Returns the sources whose files still need opening for a title and a
    /// count, which the caller hands to `job::count`.
    ///
    /// **A rescan of the same folder keeps what is already known**: titles,
    /// counts, ticks, and the outcome of this session's run — it is what runs
    /// after every analysis, and a list that blanked its figures and its
    /// selection each time would be a list nobody could work through.
    pub fn rescan(&mut self) -> Vec<Source> {
        let root = self.folder();
        let same = root.is_some() && root == self.root;
        if !same {
            self.generation += 1;
            self.selected.clear();
            // Waiting on a folder no longer on screen.
            self.uncounted.clear();
        }
        self.root = root.clone();

        let Some(root) = root else {
            self.entries.clear();
            self.status = "Choose an export folder.".into();
            return Vec::new();
        };
        let known: HashMap<PathBuf, Entry> = std::mem::take(&mut self.entries)
            .into_iter()
            .map(|e| (e.target.path().to_path_buf(), e))
            .collect();

        let mut uncounted = Vec::new();
        for target in crate::refresh::library(&root) {
            let path = target.path().to_path_buf();
            let entry = match known.get(&path).filter(|_| same) {
                Some(old) => Entry {
                    target,
                    ..old.clone()
                },
                None => {
                    uncounted.push(target.source.clone());
                    Entry {
                        title: target.name(),
                        target,
                        messages: None,
                        chats: 1,
                        chat_id: None,
                        outcome: Outcome::Idle,
                    }
                }
            };
            self.entries.push(entry);
        }
        // A new folder opens with what needs a report already ticked: that is
        // what pointing the window at a folder the exporter has just synced is
        // for, and it is one press from there.
        if !same {
            self.selected = self
                .entries
                .iter()
                .filter(|e| e.target.stale)
                .map(|e| e.target.path().to_path_buf())
                .collect();
        }
        self.selected
            .retain(|p| self.entries.iter().any(|e| e.target.path() == p));
        if !self.running {
            self.status = self.summary(&root);
        }
        uncounted
    }

    /// What the folder holds, for the status line.
    fn summary(&self, root: &Path) -> String {
        if self.entries.is_empty() {
            return if root.exists() {
                "No export in that folder.".into()
            } else {
                "That folder does not exist.".into()
            };
        }
        let behind = self.entries.iter().filter(|e| e.target.stale).count();
        let n = self.entries.len();
        // A destination holds both kinds: Classic folders beside synced
        // databases. Each is named with its own count, databases first, the
        // order the rows are listed in.
        let databases = self
            .entries
            .iter()
            .filter(|e| matches!(e.target.source, Source::Database(_)))
            .count();
        let folders = n - databases;
        let plural = |count: usize, one: &str, many: &str| match count {
            1 => format!("1 {one}"),
            c => format!("{c} {many}"),
        };
        let what = match (databases, folders) {
            (d, 0) => plural(d, "database", "databases"),
            (0, f) => plural(f, "folder export", "folder exports"),
            (d, f) => format!(
                "{} and {}",
                plural(d, "database", "databases"),
                plural(f, "folder export", "folder exports")
            ),
        };
        match behind {
            0 => format!("{what}, every report up to date."),
            b if b == n && n > 1 => format!("{what}, none reported on yet."),
            b => format!("{what}, {b} need a report."),
        }
    }

    /// The rows the list shows, in order: indices into [`State::entries`].
    pub fn visible(&self) -> Vec<usize> {
        let needle = self.filter.trim().to_lowercase();
        let mut rows: Vec<usize> = (0..self.entries.len())
            .filter(|&i| {
                let e = &self.entries[i];
                self.show.admits(e)
                    && (needle.is_empty()
                        || e.title.to_lowercase().contains(&needle)
                        || e.target.name().to_lowercase().contains(&needle))
            })
            .collect();
        let e = |i: &usize| &self.entries[*i];
        let name = |i: &usize| e(i).title.to_lowercase();
        match self.sort {
            Sort::Name => rows.sort_by_key(name),
            // A missing count sorts last rather than as zero.
            Sort::Largest => rows.sort_by(|a, b| {
                e(b).messages
                    .cmp(&e(a).messages)
                    .then_with(|| name(a).cmp(&name(b)))
            }),
            Sort::Recent => rows.sort_by(|a, b| {
                e(b).target
                    .changed
                    .cmp(&e(a).target.changed)
                    .then_with(|| name(a).cmp(&name(b)))
            }),
            Sort::Behind => rows.sort_by(|a, b| {
                e(b).target
                    .stale
                    .cmp(&e(a).target.stale)
                    .then_with(|| name(a).cmp(&name(b)))
            }),
        }
        rows
    }

    pub fn toggle(&mut self, path: &Path) {
        if !self.selected.remove(path) {
            self.selected.insert(path.to_path_buf());
        }
    }

    /// All, None, or what needs a report — **over the visible rows only**, so a
    /// tick on a row the filter is hiding is left where it is rather than
    /// silently changed.
    pub fn select(&mut self, which: Pick) {
        for i in self.visible() {
            let entry = &self.entries[i];
            let path = entry.target.path().to_path_buf();
            let on = match which {
                Pick::All => true,
                Pick::None => false,
                Pick::Behind => entry.target.stale,
            };
            if on {
                self.selected.insert(path);
            } else {
                self.selected.remove(&path);
            }
        }
    }

    /// The footer's line: how many are ticked and what they hold.
    pub fn selection_label(&self) -> String {
        let picked: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| self.selected.contains(e.target.path()))
            .collect();
        let total: usize = picked.iter().filter_map(|e| e.messages).sum();
        let uncounted = picked.iter().any(|e| e.messages.is_none());
        // The chat list's wording, `tgx_ui::selection_label`, with exports for
        // chats. Written out rather than called: this crate has no toolkit in
        // it, and `tgx-ui` is one.
        let n = picked.len();
        if n == 0 {
            return "Nothing selected".into();
        }
        let exports = if n == 1 { "export" } else { "exports" };
        let at_least = if uncounted { "at least " } else { "" };
        format!("{n} {exports}, {at_least}{} messages", thousands(total))
    }

    /// Whether Analyse does anything.
    ///
    /// Two facts and nothing else, deliberately: the same question asked two
    /// ways is how a button ends up live during a run.
    pub fn ready(&self) -> bool {
        !self.running && !self.selected.is_empty()
    }

    /// Take the ticked rows as a run, in the order the list shows them.
    ///
    /// Ticked rows the filter hides are included: a tick is the statement, the
    /// filter is only a way of looking.
    pub fn start(&mut self) -> Option<Vec<Target>> {
        if !self.ready() {
            return None;
        }
        let mut order = self.visible();
        let hidden: Vec<usize> = (0..self.entries.len())
            .filter(|i| !order.contains(i))
            .collect();
        order.extend(hidden);
        order.retain(|&i| self.selected.contains(self.entries[i].target.path()));
        let mut targets = Vec::with_capacity(order.len());
        for i in order {
            self.entries[i].outcome = Outcome::Queued;
            targets.push(self.entries[i].target.clone());
        }
        self.running = true;
        self.stopping = false;
        self.done = 0;
        self.of = targets.len();
        self.written = 0;
        self.failed.clear();
        self.status = "Starting".into();
        Some(targets)
    }

    /// Report on one row, whatever is ticked: a row's own Report action.
    ///
    /// The ticks are left as they were. They are a selection someone is
    /// building, and pressing Report on one row is not a statement about the
    /// others — except that this row, once written, is unticked like any other
    /// by [`State::apply`].
    pub fn start_one(&mut self, path: &Path) -> Option<Vec<Target>> {
        let target = self.entry_mut(path)?.target.clone();
        self.start_these(vec![target])
    }

    /// A run over these targets, listed rows or not: "report after export"
    /// hands over the chats a run has just written, and the library may be
    /// pointed somewhere else entirely. A target that is a row is marked
    /// queued like any other; one that is not is simply reported on.
    pub fn start_these(&mut self, targets: Vec<Target>) -> Option<Vec<Target>> {
        if self.running || targets.is_empty() {
            return None;
        }
        for t in &targets {
            if let Some(e) = self.entry_mut(t.path()) {
                e.outcome = Outcome::Queued;
            }
        }
        self.running = true;
        self.stopping = false;
        self.done = 0;
        self.of = targets.len();
        self.written = 0;
        self.failed.clear();
        self.status = "Starting".into();
        Some(targets)
    }

    /// How full the bar is, or `None` when nothing is running.
    pub fn fill(&self) -> Option<f32> {
        self.running
            .then(|| self.done as f32 / self.of.max(1) as f32)
    }

    fn entry_mut(&mut self, path: &Path) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.target.path() == path)
    }

    pub fn apply(&mut self, event: Progress) {
        match event {
            Progress::Step { path, label } => {
                match self.in_hand.iter_mut().find(|(p, _)| *p == path) {
                    Some((_, said)) => *said = label,
                    None => self.in_hand.push((path, label)),
                }
                self.say_in_hand();
            }
            Progress::Started(path) => {
                if let Some(e) = self.entry_mut(&path) {
                    e.outcome = Outcome::Running;
                }
            }
            Progress::Wrote { path, written } => {
                self.put_down(&path);
                self.written += 1;
                self.done += 1;
                if let Some(e) = self.entry_mut(&path) {
                    e.outcome = Outcome::Done;
                    // A folder export is counted by the run itself, which is
                    // the only time anything reads it whole.
                    e.messages = Some(written.messages);
                }
                // Done is done: the tick has been acted on, and leaving it
                // would make the next press redo it.
                self.selected.remove(&path);
            }
            Progress::Failed { path, error } => {
                self.put_down(&path);
                self.done += 1;
                let name = self
                    .entry_mut(&path)
                    .map(|e| {
                        e.outcome = Outcome::Failed(error.clone());
                        e.title.clone()
                    })
                    .unwrap_or_default();
                self.failed.push(format!("{name}: {error}"));
            }
            Progress::Finished { stopped } => {
                self.running = false;
                self.stopping = false;
                self.in_hand.clear();
                for e in &mut self.entries {
                    if e.outcome == Outcome::Queued {
                        e.outcome = Outcome::Idle;
                    }
                }
                // Re-read the folder: every report just written changes what
                // is stale, and the rows should say so. A database that
                // appeared meanwhile — the exporter finishing a chat, "report
                // after export" adding it — is a new row with no count, and
                // is kept for the window to send off to be counted.
                let found = self.rescan();
                self.uncounted.extend(found);
                self.status = finished_status(self.written, &self.failed, stopped);
            }
        }
    }

    /// A row is finished with: its line leaves the status.
    fn put_down(&mut self, path: &Path) {
        self.in_hand.retain(|(p, _)| p != path);
        if !self.in_hand.is_empty() {
            self.say_in_hand();
        }
    }

    /// The status line during a run: every row in hand, in the order they
    /// were taken. One row reads exactly as it did when there was one thread.
    /// Not while stopping — that line is the one that matters then.
    fn say_in_hand(&mut self) {
        if self.stopping {
            return;
        }
        self.status = self
            .in_hand
            .iter()
            .map(|(_, label)| label.as_str())
            .collect::<Vec<_>>()
            .join("  ·  ");
    }

    /// The rows a finished run's rescan found and nothing has counted yet,
    /// for the caller to hand to `job::count` — once no count is running,
    /// since starting one drops the last.
    ///
    /// Kept here rather than returned by [`State::apply`] because the batch
    /// that ends a run may arrive while a count from a folder pick is still
    /// going, and a list returned then would have nowhere to wait.
    pub fn take_uncounted(&mut self) -> Vec<Source> {
        std::mem::take(&mut self.uncounted)
    }

    pub fn apply_count(&mut self, counted: Counted) {
        if counted.generation != self.generation {
            return;
        }
        let Some(e) = self.entry_mut(&counted.path) else {
            return;
        };
        if let Some(title) = counted.title {
            e.title = title;
        }
        e.chat_id = counted.chat_id;
        e.messages = counted.messages;
        e.chats = counted.chats.max(1);
        if let Some(error) = counted.error {
            e.outcome = Outcome::Failed(error);
        }
    }
}

/// Which rows a selection verb ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    All,
    None,
    Behind,
}

/// What the status line says when a run ends.
///
/// A failure is named, with the first one's reason: "1 failed" alone sends
/// somebody through forty rows to find which.
pub fn finished_status(written: usize, failed: &[String], stopped: bool) -> String {
    let reports = if written == 1 { "report" } else { "reports" };
    let head = if stopped {
        format!("Stopped after {written} {reports}")
    } else {
        format!("Wrote {written} {reports}")
    };
    match failed {
        [] => format!("{head}."),
        [first, ..] => format!("{head}; {} failed — {first}", failed.len()),
    }
}

/// `2.4 GB`. Decimal units, as Explorer's own column is not and as the
/// exporter's list is.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// When, in as few words as are true: a time today, `yesterday`, a day this
/// year, or a full date.
pub fn human_when(at: SystemTime, now: SystemTime) -> String {
    let at: DateTime<Local> = at.into();
    let now: DateTime<Local> = now.into();
    let (day, today) = (at.date_naive(), now.date_naive());
    if day == today {
        at.format("today %H:%M").to_string()
    } else if today.pred_opt() == Some(day) {
        "yesterday".into()
    } else if at.format("%Y").to_string() == now.format("%Y").to_string() {
        at.format("%-d %b").to_string()
    } else {
        at.format("%-d %b %Y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refresh::Written;
    use std::time::Duration;

    fn entry(name: &str, messages: Option<usize>, stale: bool, reported: bool) -> Entry {
        let path = PathBuf::from(format!("{name}/{name}.sqlite"));
        Entry {
            target: Target {
                out: PathBuf::from(format!("{name}/report.html")),
                source: Source::Database(path),
                stale,
                changed: None,
                reported: reported.then_some(SystemTime::UNIX_EPOCH),
                bytes: 0,
            },
            title: name.into(),
            messages,
            chats: 1,
            chat_id: None,
            outcome: Outcome::Idle,
        }
    }

    fn library() -> State {
        State {
            entries: vec![
                entry("Redakcija", Some(41_870), false, true),
                entry("UA KOLAB", Some(8_206), true, true),
                entry("Admini", None, true, false),
            ],
            ..Default::default()
        }
    }

    fn titles(state: &State) -> Vec<String> {
        state
            .visible()
            .into_iter()
            .map(|i| state.entries[i].title.clone())
            .collect()
    }

    #[test]
    fn what_needs_a_report_comes_first_by_default() {
        assert_eq!(titles(&library()), ["Admini", "UA KOLAB", "Redakcija"]);
    }

    #[test]
    fn an_uncounted_export_sorts_after_every_counted_one_not_as_zero() {
        let mut state = library();
        state.sort = Sort::Largest;
        assert_eq!(titles(&state), ["Redakcija", "UA KOLAB", "Admini"]);
    }

    #[test]
    fn the_filter_and_the_show_menu_narrow_together() {
        let mut state = library();
        state.show = Show::Behind;
        assert_eq!(titles(&state), ["Admini", "UA KOLAB"]);
        state.filter = "  kolab ".into();
        assert_eq!(titles(&state), ["UA KOLAB"]);
    }

    #[test]
    fn a_row_says_the_one_thing_that_matters_most_about_it() {
        let state = library();
        let marks: Vec<Mark> = state.entries.iter().map(Entry::mark).collect();
        assert_eq!(marks, [Mark::Current, Mark::Changed, Mark::NoReport]);
        let mut running = state.entries[0].clone();
        running.outcome = Outcome::Running;
        assert_eq!(running.mark(), Mark::Running, "a run outranks the report");
    }

    #[test]
    fn selection_verbs_touch_only_what_the_filter_shows() {
        let mut state = library();
        state
            .selected
            .insert(PathBuf::from("Redakcija/Redakcija.sqlite"));
        state.filter = "admini".into();
        state.select(Pick::All);
        assert_eq!(state.selected.len(), 2, "the hidden tick stayed");
        state.filter.clear();
        state.select(Pick::Behind);
        assert_eq!(state.selected.len(), 2);
        assert!(!state
            .selected
            .contains(Path::new("Redakcija/Redakcija.sqlite")));
    }

    #[test]
    fn the_footer_counts_exports_and_says_at_least_when_any_is_uncounted() {
        let mut state = library();
        state.select(Pick::Behind);
        assert_eq!(
            state.selection_label(),
            "2 exports, at least 8,206 messages"
        );
        state.select(Pick::None);
        assert_eq!(state.selection_label(), "Nothing selected");
    }

    #[test]
    fn a_run_takes_the_ticks_and_the_bar_counts_exports() {
        let mut state = library();
        assert!(!state.ready(), "nothing ticked");
        state.select(Pick::Behind);
        let run = state.start().expect("a run");
        assert_eq!(run.len(), 2);
        assert!(!state.ready(), "a run is going");
        assert_eq!(state.fill(), Some(0.0));

        let path = run[0].path().to_path_buf();
        state.apply(Progress::Started(path.clone()));
        assert_eq!(state.entries[2].mark(), Mark::Running);
        state.apply(Progress::Wrote {
            path: path.clone(),
            written: Written {
                out: run[0].out.clone(),
                messages: 12,
                topics: 1,
                people: 2,
                events: 0,
                warning: None,
            },
        });
        assert_eq!(state.fill(), Some(0.5));
        assert!(!state.selected.contains(&path), "done is unticked");
        assert_eq!(state.entries[2].messages, Some(12), "counted by the run");
    }

    #[test]
    fn rows_read_at_once_share_the_status_and_leave_it_as_they_finish() {
        let mut state = library();
        state.select(Pick::Behind);
        let run = state.start().expect("a run");
        let (a, b) = (run[0].path().to_path_buf(), run[1].path().to_path_buf());
        for (path, label) in [(&a, "1/2  A"), (&b, "2/2  B"), (&a, "1/2  A: reading x")] {
            state.apply(Progress::Step {
                path: path.clone(),
                label: label.into(),
            });
        }
        assert_eq!(state.status, "1/2  A: reading x  ·  2/2  B");
        state.apply(Progress::Failed {
            path: a,
            error: "no messages in it".into(),
        });
        assert_eq!(state.status, "2/2  B");
        assert_eq!(state.done, 1, "a step never moves the count");
    }

    #[test]
    fn a_failed_row_says_why_and_the_run_carries_on() {
        let mut state = library();
        state.select(Pick::Behind);
        let run = state.start().expect("a run");
        state.apply(Progress::Failed {
            path: run[0].path().to_path_buf(),
            error: "no messages in it".into(),
        });
        let failed = state
            .entries
            .iter()
            .find(|e| e.target.path() == run[0].path())
            .expect("row");
        assert_eq!(failed.mark(), Mark::Failed);
        assert_eq!(failed.caption(SystemTime::now()), "no messages in it");
        assert!(state.running, "one failure does not end the run");
    }

    #[test]
    fn the_end_of_a_run_is_named_with_its_first_failure() {
        assert_eq!(finished_status(3, &[], false), "Wrote 3 reports.");
        assert_eq!(finished_status(1, &[], true), "Stopped after 1 report.");
        assert_eq!(
            finished_status(1, &["Admini: no messages in it".into()], false),
            "Wrote 1 report; 1 failed — Admini: no messages in it"
        );
    }

    #[test]
    fn a_count_from_a_folder_no_longer_on_screen_is_dropped() {
        let mut state = library();
        state.generation = 2;
        state.apply_count(Counted {
            generation: 1,
            path: PathBuf::from("Admini/Admini.sqlite"),
            title: Some("Admini x koordinatori".into()),
            chat_id: Some(-1_007),
            messages: Some(5),
            chats: 1,
            error: None,
        });
        assert_eq!(state.entries[2].messages, None);
        assert_eq!(state.entries[2].chat_id, None);
        state.apply_count(Counted {
            generation: 2,
            path: PathBuf::from("Admini/Admini.sqlite"),
            title: Some("Admini x koordinatori".into()),
            chat_id: Some(-1_007),
            messages: Some(5),
            chats: 1,
            error: None,
        });
        assert_eq!(state.entries[2].messages, Some(5));
        assert_eq!(state.entries[2].title, "Admini x koordinatori");
        assert_eq!(state.entries[2].chat_id, Some(-1_007));
    }

    #[test]
    fn one_rows_report_leaves_the_other_ticks_alone() {
        let mut state = library();
        state.selected = state
            .entries
            .iter()
            .map(|e| e.target.path().to_path_buf())
            .collect();
        let admini = PathBuf::from("Admini/Admini.sqlite");
        let targets = state.start_one(&admini).expect("idle, and the row exists");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path(), admini);
        assert_eq!(state.of, 1);
        assert_eq!(state.entries[2].outcome, Outcome::Queued);
        assert_eq!(state.entries[0].outcome, Outcome::Idle);
        assert_eq!(state.selected.len(), 3, "the selection is not the run");
        // One at a time: a second press while it runs does nothing.
        assert!(state.start_one(&admini).is_none());
    }

    #[test]
    fn a_row_that_is_not_there_starts_nothing() {
        let mut state = library();
        assert!(state.start_one(Path::new("Gone/Gone.sqlite")).is_none());
        assert!(!state.running);
    }

    #[test]
    fn sizes_and_dates_are_short() {
        assert_eq!(human_bytes(94_208), "94 KB");
        assert_eq!(human_bytes(2_589_196_288), "2.6 GB");
        assert_eq!(human_bytes(12), "12 B");
        let now = SystemTime::now();
        assert!(human_when(now, now).starts_with("today "));
        assert_eq!(
            human_when(now - Duration::from_secs(86_400), now),
            "yesterday"
        );
    }

    #[test]
    fn a_path_pasted_out_of_explorer_keeps_its_quotes_off() {
        // "Copy as path" wraps the whole thing in double quotes, and a path with
        // them still attached is a folder that does not exist.
        let mut state = State {
            path: "  \"N:\\telegram export\\UA KOLAB\"  ".into(),
            ..Default::default()
        };
        assert_eq!(
            state.folder(),
            Some(PathBuf::from(r"N:\telegram export\UA KOLAB"))
        );
        state.path = "   ".into();
        assert_eq!(state.folder(), None, "whitespace is not a folder");
    }

    #[test]
    fn a_new_folder_ticks_what_needs_a_report_and_forgets_the_old_ticks() {
        let base = std::env::temp_dir().join("tga-app-state-scan");
        let _ = std::fs::remove_dir_all(&base);
        for name in ["A", "B"] {
            let dir = base.join(name);
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(dir.join(format!("{name}.sqlite")), b"").expect("touch");
        }
        std::fs::write(base.join("B").join("report.html"), b"").expect("touch");

        let mut state = State::default();
        state.selected.insert(PathBuf::from("elsewhere"));
        let uncounted = state.set_path(&base);
        assert_eq!(uncounted.len(), 2, "both files need opening");
        assert_eq!(state.entries.len(), 2);
        assert_eq!(state.selected.len(), 1, "only A has no report");
        assert!(state.status.contains("2 databases"), "{}", state.status);

        // The same folder again keeps the ticks and asks for nothing new.
        state.toggle(&base.join("B").join("B.sqlite"));
        assert!(state.rescan().is_empty());
        assert_eq!(state.selected.len(), 2);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_database_that_appears_during_a_run_is_left_to_be_counted() {
        let base =
            std::env::temp_dir().join(format!("tga-run-state-appears-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let touch = |name: &str| {
            let dir = base.join(name);
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(dir.join(format!("{name}.sqlite")), b"").expect("touch");
        };
        touch("A");
        let mut state = State::default();
        assert_eq!(state.set_path(&base).len(), 1);
        state.start().expect("A needs a report");

        // The exporter finishes another chat while the report is written.
        touch("C");
        state.apply(Progress::Finished { stopped: false });
        assert_eq!(
            state.take_uncounted(),
            vec![Source::Database(base.join("C").join("C.sqlite"))]
        );
        assert!(state.take_uncounted().is_empty(), "handed over once");

        // One found and then left behind by a folder change is not counted.
        state.start_one(&base.join("A").join("A.sqlite"));
        touch("D");
        state.apply(Progress::Finished { stopped: false });
        state.set_path(&std::env::temp_dir().join("tga-run-state-elsewhere"));
        assert!(state.take_uncounted().is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }
}

//! The interaction rules, driven without a window.
//!
//! [`Shell::headless`] exists for this: every rule below is behaviour, not
//! painting, and each one is a bug that was found the expensive way.

use super::*;
use crate::bridge::{Activity, Event};
use crate::queue::JobState;
use tgx_tg::client::ChatKind;
use tgx_ui::components::{selection_label, ListState};

fn chat(id: i64, title: &str, count: Option<i64>) -> ChatInfo {
    ChatInfo {
        id,
        title: title.into(),
        kind: ChatKind::Supergroup,
        last_activity: 0,
        is_forum: false,
        public: false,
        message_count: count,
        migration: None,
    }
}

fn shell_with(chats: Vec<ChatInfo>) -> Shell {
    let mut s = Shell::headless();
    s.signed_in = true;
    s.loaded = true;
    s.chats = chats;
    s.rebuild_rows();
    s
}

#[test]
fn the_list_opens_on_not_signed_in() {
    // An empty list is the state this app opens on, and blank is not
    // neutral: it reads as broken.
    let s = Shell::headless();
    assert_eq!(s.list_state(), ListState::NotSignedIn);
    assert!(s.list_state().empty_state("").is_some());
}

#[test]
fn signed_in_with_nothing_loaded_says_so() {
    let mut s = Shell::headless();
    s.signed_in = true;
    assert_eq!(s.list_state(), ListState::SignedInNothingLoaded);
}

#[test]
fn a_filter_matching_nothing_is_not_an_empty_account() {
    let mut s = shell_with(vec![chat(1, "news", None)]);
    // The filter and the rows are rebuilt together — the count the empty state
    // reads is cached with them, so changing one without the other is how a
    // list ends up painting rows under "Nothing matches".
    s.view.filter = "zzz".into();
    s.rebuild_rows();
    assert_eq!(s.list_state(), ListState::FilterMatchedNothing);
    s.view.filter.clear();
    s.rebuild_rows();
    assert_eq!(s.list_state(), ListState::Populated);
}

#[test]
fn selection_actions_are_disabled_over_an_empty_list() {
    // Nothing offers to do what it cannot.
    let s = Shell::headless();
    assert!(!s.selection_actions_enabled());
    let populated = shell_with(vec![chat(1, "a", None)]);
    assert!(populated.selection_actions_enabled());
}

#[test]
fn ticks_are_held_by_id_so_they_survive_filtering() {
    let mut s = shell_with(vec![chat(1, "news", None), chat(2, "other", None)]);
    s.view.filter = "news".into();
    s.selected.insert(1);
    // Filter to something else: the tick is still held.
    s.view.filter = "other".into();
    assert!(s.selected.contains(&1));
    s.view.filter.clear();
    assert_eq!(s.visible().len(), 2);
    assert!(s.selected.contains(&1));
}

#[test]
fn the_footer_says_at_least_when_a_selected_chat_is_uncounted() {
    let mut s = shell_with(vec![chat(1, "a", Some(10)), chat(2, "b", None)]);
    s.selected.insert(1);
    let (total, any) = s.selection_total();
    assert_eq!((total, any), (10, false));
    s.selected.insert(2);
    let (total, any) = s.selection_total();
    assert_eq!((total, any), (10, true));
    assert!(selection_label(2, total, any).contains("at least"));
}

#[test]
fn a_zero_count_is_summed_and_a_missing_one_is_not() {
    let mut s = shell_with(vec![chat(1, "empty", Some(0)), chat(2, "unknown", None)]);
    s.selected.insert(1);
    assert_eq!(s.selection_total(), (0, false), "0 is a count");
    s.selected.clear();
    s.selected.insert(2);
    assert_eq!(s.selection_total(), (0, true), "blank is not a count");
}

#[test]
fn an_unreadable_theme_setting_still_opens_a_readable_window() {
    let p = Palette::named("chartreuse");
    assert_eq!(p, Palette::dark());
}

#[test]
fn the_appearance_control_offers_exactly_the_two_palettes_that_exist() {
    // The control was a chip on the nav bar naming the appearance it switched
    // *to*; it is a Light / Dark strip in Settings now. What has to hold is
    // unchanged: every name the strip can hand to `set_theme` must produce the
    // palette it names, or the control is a label that lies. Asserted on the
    // names rather than by driving `set_theme`, which writes settings.json.
    assert_eq!(Palette::named(THEMES[0]), Palette::light());
    assert_eq!(Palette::named(THEMES[1]), Palette::dark());
    assert_ne!(Palette::named(THEMES[0]), Palette::named(THEMES[1]));
}

// -- the count, and its one writer ----------------------------------------

#[test]
fn a_count_reaches_the_row_the_list_paints_and_sorts_on() {
    // Three sources write a count and there is one setter, or a finished
    // export leaves the row showing one number and sorting on another.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.apply(Event::Counted {
        chat_id: 1,
        total: Some(6643),
    });
    assert_eq!(s.chats[0].message_count, Some(6643));
    assert_eq!(s.count_of(1), Some(6643));
}

#[test]
fn a_chat_telegram_would_not_count_stays_blank_rather_than_zero() {
    let mut s = shell_with(vec![chat(1, "a", Some(5))]);
    s.apply(Event::Counted {
        chat_id: 1,
        total: None,
    });
    assert_eq!(s.chats[0].message_count, None, "None is not zero");
}

#[test]
fn an_export_fills_in_a_count_nobody_asked_for() {
    // The export looks the total up anyway; writing it to the list is free,
    // and without it the row reads blank beside a progress bar that plainly
    // knows the answer.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.queue.start([(1, "a".to_string())]);
    s.apply(Event::ChatTotal {
        chat_id: 1,
        total: 900,
    });
    assert_eq!(s.count_of(1), Some(900));
}

#[test]
fn a_looked_up_total_does_not_overwrite_a_measured_one() {
    // The Count button's answer came from the same request; what must not
    // happen is the pre-export estimate replacing what an export wrote.
    let mut s = shell_with(vec![chat(1, "a", Some(6643))]);
    s.queue.start([(1, "a".to_string())]);
    s.apply(Event::ChatTotal {
        chat_id: 1,
        total: 10,
    });
    assert_eq!(s.count_of(1), Some(6643));
}

#[test]
fn what_an_export_wrote_replaces_what_the_list_was_carrying() {
    let mut s = shell_with(vec![chat(1, "a", Some(6643))]);
    s.queue.start([(1, "a".to_string())]);
    s.apply(Event::ChatDone {
        messages_measured: true,
        chat_id: 1,
        messages: 6640,
        expected: 6643,
        topics: Some(4),
        media_downloaded: 830,
        media_failed: 6,
        root: Some(std::path::PathBuf::from("out")),
    });
    assert_eq!(s.count_of(1), Some(6640), "a measured number wins");
    assert_eq!(s.queue.jobs()[0].state, JobState::Done);
}

#[test]
fn a_failed_export_writes_no_count_at_all() {
    // A truncated run must not leave its own length behind as the size of
    // the chat.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.queue.start([(1, "a".to_string())]);
    s.apply(Event::ChatFailed {
        chat_id: 1,
        message: "CHAT_ADMIN_REQUIRED".into(),
    });
    assert_eq!(s.count_of(1), None);
    assert_eq!(s.journal.warnings(), 1);
}

// -- one bar, two claimants -----------------------------------------------

#[test]
fn a_count_finishing_mid_export_does_not_touch_the_export_s_progress() {
    // "Counted 12 of 12 chats" painted over "6,000 of 6,643" is the failure.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.exporting = true;
    s.status = "Exporting a".into();
    s.apply(Event::CountProgress {
        done: 12,
        total: 12,
    });
    assert_eq!(s.count_progress, None);
    assert_eq!(s.status.as_str(), "Exporting a");
    s.apply(Event::CountFinished {
        counted: 12,
        failed: 0,
    });
    assert_eq!(s.status.as_str(), "Exporting a");
}

#[test]
fn a_rate_limit_does_not_take_the_status_line_for_the_rest_of_the_chat() {
    // Nothing else writes the status during an export, so one 60-second wait
    // left the bar reading "Rate limited, waiting 60s" while progress
    // advanced beside it.
    let mut s = shell_with(vec![chat(1, "news", None)]);
    s.exporting = true;
    s.queue.start([(1, "news".to_string())]);
    s.apply(Event::FloodWait(60));
    assert!(s.status.as_str().contains("Rate limited"));
    s.apply(Event::Progress {
        chat_id: 1,
        done: 200,
        total: 6643,
    });
    assert_eq!(s.status.as_str(), "Exporting news");
}

#[test]
fn a_run_that_could_not_start_says_why_and_not_only_how_many() {
    // `Failed` and `Finished` arrive in the same batch when the destination
    // is unusable, and the queue's tally used to land on top of the one
    // sentence naming the cause.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.exporting = true;
    s.queue.start([(1, "a".to_string())]);
    s.apply(Event::Failed {
        activity: Activity::Export,
        message: "cannot write into D:\\gone".into(),
    });
    s.apply(Event::Finished { stopped: true });
    assert!(
        s.status.as_str().contains("cannot write into"),
        "got {}",
        s.status
    );
    // And the cause is spent, so the next clean run is not haunted by it.
    assert!(s.failure.is_none());
}

#[test]
fn a_sign_in_failing_mid_export_does_not_stop_the_export() {
    // `Failed` was a global switch: any sender cleared `exporting` and
    // `counting`, and five senders share it. A sign-in probe that could not
    // reach Telegram while an export ran made the window believe the export had
    // stopped — the Stop button vanished, the bar froze — while the export went
    // on writing files until its own `Finished` put the state back.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.exporting = true;
    s.counting = true;
    s.apply(Event::Failed {
        activity: Activity::SignIn,
        message: "Telegram did not answer".into(),
    });
    assert!(s.exporting, "a sign-in failure is not an export failure");
    assert!(s.counting, "nor a count failure");
    // It is still reported.
    assert!(s.status.as_str().contains("did not answer"));
    // And it is not the export's stated cause, so it cannot reach the run's
    // summary line.
    assert!(s.failure.is_none());

    // The count's own failure does stop the count, and only the count.
    s.apply(Event::Failed {
        activity: Activity::Count,
        message: "rate limited".into(),
    });
    assert!(!s.counting);
    assert!(s.exporting);
}

#[test]
fn a_failure_from_the_last_run_does_not_reach_the_next_ones_summary() {
    // `failure` is appended to the run's summary and was only ever cleared by
    // the `Finished` that consumed it — so a run that ended some other way left
    // its cause behind to be reported as the reason the *next* run ended.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.failure = Some("cannot write into D:\\gone".into());
    s.begin_run();
    assert!(s.failure.is_none(), "a stale cause survived into a new run");
}

#[test]
fn a_failed_chat_says_which_chat() {
    // The transcript prefixes every other line with the chat. This one did not,
    // so "no messages could be read" in a twenty-chat queue named none of them
    // — and the queue table that does know is a different panel, sorted
    // differently.
    let mut s = shell_with(vec![chat(1, "news", None), chat(2, "other", None)]);
    s.queue
        .start([(1, "news".to_string()), (2, "other".to_string())]);
    s.apply(Event::ChatFailed {
        chat_id: 2,
        message: "no messages could be read".into(),
    });
    let last = s.journal.to_text();
    assert!(last.contains("other: no messages could be read"), "{last}");
}

#[test]
fn a_refresh_keeps_the_counts_it_took_minutes_to_fetch() {
    // The dialog list carries no totals, so replacing it wholesale silently
    // undid a Count that had spent one request per chat to get them.
    let mut s = shell_with(vec![chat(1, "a", Some(6643)), chat(2, "b", Some(0))]);
    s.apply(Event::Chats(vec![
        chat(1, "a", None),
        chat(2, "b", None),
        chat(3, "new", None),
    ]));
    assert_eq!(s.count_of(1), Some(6643));
    assert_eq!(s.count_of(2), Some(0), "0 is a count and survives too");
    assert_eq!(s.count_of(3), None, "a chat nobody counted stays blank");
}

#[test]
fn a_chat_that_was_never_split_reports_no_topic_count() {
    // The engine counts its single General sink as one output folder, so a
    // private chat's em dash flipped to `1` the moment it finished.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.queue.start([(1, "a".to_string())]);
    s.apply(Event::ChatDone {
        messages_measured: true,
        chat_id: 1,
        messages: 250,
        expected: 250,
        topics: None,
        media_downloaded: 12,
        media_failed: 0,
        root: Some(std::path::PathBuf::from("out")),
    });
    assert_eq!(s.queue.jobs()[0].topics_text(), "\u{2014}");
}

// -- stop actually stops ---------------------------------------------------

#[test]
fn stop_raises_the_flag_the_worker_reads() {
    // The old Stop set `exporting = false`, cleared the bar and wrote
    // "Stopped" while the export ran to completion writing files.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.exporting = true;
    s.stop();
    assert!(s.cancel.is_cancelled(), "nothing told the worker");
    assert!(
        s.exporting,
        "the run is over when the worker says so, not when the button is pressed"
    );
    assert_eq!(s.status.as_str(), "Stopping…");
}

#[test]
fn a_stopped_run_is_not_reported_as_a_success() {
    // `Finished` used to carry its own sentence and overwrote "Stopped" with
    // "Exported 3 of 3 chats" a moment later.
    let mut s = shell_with(vec![chat(1, "a", None), chat(2, "b", None)]);
    s.exporting = true;
    s.queue.start([(1, "a".to_string()), (2, "b".to_string())]);
    s.apply(Event::ChatDone {
        messages_measured: true,
        chat_id: 1,
        messages: 5,
        expected: 5,
        topics: None,
        media_downloaded: 0,
        media_failed: 0,
        root: Some(std::path::PathBuf::from("out")),
    });
    s.stop();
    s.apply(Event::Finished { stopped: true });
    assert!(!s.exporting);
    assert_eq!(s.queue.jobs()[1].state, JobState::Paused);
    assert!(s.status.as_str().contains("1 paused"), "got {}", s.status);
}

// -- the queue: clearing, adding, removing ---------------------------------

#[test]
fn a_stop_keeps_the_rows_and_only_the_user_clears_them() {
    // Stop used to mark the rest Stopped and empty the work list, so carrying
    // on meant ticking every chat again. The rows wait now; the work list is
    // rebuilt from them on resume, so nothing runs until it is asked for.
    let mut s = shell_with(vec![chat(1, "a", None), chat(2, "b", None)]);
    s.exporting = true;
    s.queue.start([(1, "a".to_string()), (2, "b".to_string())]);
    s.pending.reset(s.chats.clone());
    s.queue.began(1);
    s.stop();
    s.apply(Event::Finished { stopped: true });
    assert!(s.pending.is_empty(), "nothing runs until Resume");
    assert_eq!(s.queue.len(), 2);
    assert!(s.queue.is_paused());
    assert_eq!(
        s.queue.jobs()[0].state,
        JobState::Paused,
        "the interrupted one"
    );

    // Clear is the user's, and only between runs.
    s.exporting = true;
    s.clear_queue();
    assert_eq!(s.queue.len(), 2, "no clearing mid-run");
    s.exporting = false;
    s.clear_queue();
    assert!(s.queue.is_empty());
}

#[test]
fn a_chat_taken_out_of_the_queue_leaves_both_lists() {
    // The two have to move together: out of the table alone and the export
    // writes the chat with no row reporting it; out of the work list alone and
    // a row says Queued for a chat that is never coming.
    let mut s = shell_with(vec![chat(1, "a", None), chat(2, "b", None)]);
    s.exporting = true;
    s.queue.start([(1, "a".to_string()), (2, "b".to_string())]);
    s.pending.reset(s.chats.clone());
    s.remove_from_queue(2);
    assert_eq!(s.queue.len(), 1);
    assert_eq!(
        s.pending
            .snapshot()
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        vec![1]
    );
}

#[test]
fn a_chat_taken_out_of_the_queue_does_not_come_back_on_the_next_start() {
    // Shipped broken. The × removed the row and left the tick in the chat
    // list, and the tick is what `start_export` reads — so the removal was
    // undone by the very next press of the button beside it.
    let mut s = shell_with(vec![chat(1, "a", None), chat(2, "b", None)]);
    s.selected.insert(1);
    s.selected.insert(2);
    s.queue.start([(1, "a".to_string()), (2, "b".to_string())]);
    s.pending.reset(s.chats.clone());
    s.remove_from_queue(2);
    assert!(!s.selected.contains(&2), "the tick outlived the row");
    assert!(s.selected.contains(&1), "and only that chat's tick went");
}

#[test]
fn the_chat_in_flight_keeps_its_tick_because_it_keeps_its_row() {
    // The other half: a removal that is refused must not half-happen. The
    // export is writing this chat, so unticking it would be the interface
    // disagreeing with what the run is doing.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.selected.insert(1);
    s.exporting = true;
    s.queue.start([(1, "a".to_string())]);
    s.queue.began(1);
    s.remove_from_queue(1);
    assert!(s.selected.contains(&1));
    assert_eq!(s.queue.len(), 1);
}

#[test]
fn the_chat_in_flight_cannot_be_taken_out_of_the_queue() {
    // Stop is that row's control. `Queue::remove` refuses it, and the × is not
    // painted on it either — two statements of one rule, because the second is
    // a painting decision and the first has to hold whatever the panel does.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.exporting = true;
    s.queue.start([(1, "a".to_string())]);
    s.queue.began(1);
    s.remove_from_queue(1);
    assert_eq!(s.queue.len(), 1);
    assert_eq!(s.queue.jobs()[0].state, JobState::Exporting);
}

#[test]
fn a_worker_that_ends_with_work_still_waiting_asks_for_another() {
    // The one-frame race Add to queue opens: `exporting` is cleared when the
    // worker's `Finished` is *applied*, a frame after it stopped draining, so a
    // chat pushed in that window lands on a list nobody is reading. Without the
    // resume its row says Queued for the rest of the session.
    //
    // The **decision** is what is checked, not the spawn: starting a worker
    // opens a connection to Telegram, so a test that drove `Finished` through
    // to the end would be a test that goes to the network — which nothing in
    // this file does. See `actions::export`, which no test here reaches either.
    let s = shell_with(vec![chat(1, "a", None)]);
    s.pending.reset(s.chats.clone());
    assert!(s.should_resume());
    // A stop outranks it. A fresh worker would start and immediately cancel,
    // which is a run in the log that never was.
    s.cancel.cancel();
    assert!(!s.should_resume());
}

#[test]
fn a_finished_run_with_nothing_waiting_really_does_finish() {
    // The other half of the rule above, driven all the way through because
    // nothing is queued and so nothing is spawned. A resume that fired on an
    // empty list would restart the worker after every single run.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.exporting = true;
    s.queue.start([(1, "a".to_string())]);
    assert!(!s.should_resume(), "nothing is waiting");
    s.apply(Event::Finished { stopped: false });
    assert!(!s.exporting);
    assert!(s.status.as_str().contains("Exported"), "got {}", s.status);
}

#[test]
fn starting_a_run_clears_a_cancel_left_over_from_the_last_one() {
    // Otherwise the second export stops on its first message, having been
    // told to before it began.
    let s = shell_with(vec![chat(1, "a", None)]);
    s.cancel.cancel();
    s.cancel.reset();
    assert!(!s.cancel.is_cancelled());
}

// -- the log ---------------------------------------------------------------

#[test]
fn a_warning_survives_the_chats_that_finish_after_it() {
    // Six more finished chats used to put the INCOMPLETE line out of reach:
    // only the last six were painted and nothing scrolled.
    let mut s = Shell::headless();
    s.apply(Event::Warn("INCOMPLETE".into()));
    for i in 0..50 {
        s.apply(Event::Log(format!("chat {i}: done")));
    }
    assert_eq!(s.journal.warnings(), 1);
    assert!(s.journal.lines().any(|l| l.text == "INCOMPLETE"));
}

// -- the list --------------------------------------------------------------

#[test]
fn the_painted_rows_follow_the_filter_without_touching_the_ticks() {
    let mut s = shell_with(vec![chat(1, "news", None), chat(2, "other", None)]);
    s.selected.insert(1);
    s.view.filter = "news".into();
    s.rebuild_rows();
    let chats: Vec<&ChatInfo> = s
        .rows
        .iter()
        .filter_map(|r| match r {
            PaintedRow::Chat(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0].title, "news");
    assert!(s.selected.contains(&1));
}

#[test]
fn a_refresh_drops_ticks_for_chats_that_vanished() {
    let mut s = shell_with(vec![chat(1, "a", None), chat(2, "b", None)]);
    s.selected.insert(1);
    s.selected.insert(2);
    s.apply(Event::Chats(vec![chat(2, "b", None)]));
    assert!(!s.selected.contains(&1));
    assert!(s.selected.contains(&2));
}

#[test]
fn counting_a_large_account_rebuilds_the_rows_once_not_once_per_chat() {
    // Re-sorting the whole list on every `Counted` is quadratic work for a
    // picture nobody sees until the batch is done.
    let mut s = shell_with(
        (1..=50)
            .map(|i| chat(i, &format!("chat {i}"), None))
            .collect(),
    );
    for i in 1..=50 {
        s.apply(Event::Counted {
            chat_id: i,
            total: Some(i * 10),
        });
    }
    assert!(s.rows_stale, "the rows should still be waiting to rebuild");
    s.rebuild_rows_if_stale();
    assert!(!s.rows_stale);
    assert_eq!(s.count_of(50), Some(500));
}

#[test]
fn a_folded_category_is_remembered_across_a_restart() {
    // Someone who folds Bots away has said something about how they want the
    // list to look; asking again on every launch is forgetting on purpose.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.view.folded.insert(Category::Bots);
    s.settings.folded_categories = Category::ALL
        .into_iter()
        .filter(|c| s.view.folded.contains(c))
        .map(|c| c.key().to_string())
        .collect();
    assert_eq!(s.settings.folded_categories, vec!["bot"]);

    // And a name from a build that had a category this one does not is
    // dropped rather than kept.
    let restored: std::collections::HashSet<Category> = ["bot", "sasquatch"]
        .iter()
        .filter_map(|name| Category::ALL.into_iter().find(|c| c.key() == *name))
        .collect();
    assert_eq!(restored, s.view.folded);
}

#[test]
fn the_sort_and_grouping_are_taken_from_the_saved_settings() {
    // An unknown key falls back rather than breaking, the same way the theme
    // does — an edited settings file must not leave the list unsorted.
    let mut s = Shell::headless();
    s.view.sort = SortMode::from_key("largest");
    assert_eq!(s.view.sort, SortMode::Largest);
    assert_eq!(SortMode::from_key("nonsense"), SortMode::Recent);
}

#[test]
fn export_as_classic_reports_where_it_wrote_and_comes_back() {
    let mut s = shell_with(vec![]);
    s.viewer.unpacking = true;
    s.apply(Event::Unpacked(Ok(vec![
        "Wrote Dev to X: 3 messages".into()
    ])));
    assert!(!s.viewer.unpacking, "the button comes back");
    assert_eq!(s.status, "Wrote Dev to X: 3 messages");
    s.viewer.unpacking = true;
    s.apply(Event::Unpacked(Err("Export as Classic failed: x".into())));
    assert!(!s.viewer.unpacking);
    assert_eq!(s.journal.warnings(), 1);
}

#[test]
fn a_folder_check_logs_problems_as_warnings_and_shows_the_log() {
    let mut s = shell_with(vec![]);
    s.checking = true;
    s.apply(Event::Checked(Ok(vec![
        "PROBLEM: x: holds 2 chats".into(),
        "note: y: has never finished a sync".into(),
    ])));
    assert!(!s.checking, "the button comes back");
    assert_eq!(s.journal.warnings(), 1, "only the problem is a warning");
    assert!(s.journal.to_text().contains("never finished a sync"));
    assert_eq!(s.body, View::Queue);
}

#[test]
fn a_duplicate_report_is_logged_without_warnings_and_shows_the_log() {
    // A second copy is space, not damage: none of it may land in Copy
    // warnings, where it would sit beside an INCOMPLETE export.
    let mut s = shell_with(vec![]);
    s.deduping = true;
    s.apply(Event::Dupes(Ok(vec![
        "3 file(s) in export folders and 4 blob(s) in databases: 1 held more than once".into(),
        "  2 copies of 1.0 MB, 1.0 MB extra:".into(),
        "    A.sqlite: photos 10".into(),
    ])));
    assert!(!s.deduping, "the button comes back");
    assert_eq!(s.journal.warnings(), 0);
    assert!(s.journal.to_text().contains("photos 10"));
    assert_eq!(s.body, View::Queue);

    // A folder that could not be read is the one thing that is a warning.
    s.deduping = true;
    s.apply(Event::Dupes(Err("could not read X: denied".into())));
    assert!(!s.deduping);
    assert_eq!(s.journal.warnings(), 1);
}

#[test]
fn the_scheduled_sync_fires_when_due_and_yields_to_a_run() {
    use std::time::{Duration, Instant};
    let mut s = shell_with(vec![chat(1, "a", None)]);
    let t0 = Instant::now();
    // Off: 0 never arms.
    s.arm_schedule(t0);
    assert_eq!(s.tick_schedule(t0 + Duration::from_secs(1_000_000)), None);

    s.settings.export_db = true;
    s.settings.sync_every_hours = 2;
    // As the panel would: Update databases commits the form first.
    s.form.sync(&s.settings);
    s.arm_schedule(t0);
    let hour = Duration::from_secs(3600);
    assert_eq!(s.tick_schedule(t0 + hour), Some(hour), "not yet");
    assert!(!s.scanning);

    // Due while a run is going: looks again in a minute, starts nothing.
    s.exporting = true;
    assert_eq!(
        s.tick_schedule(t0 + 2 * hour),
        Some(Duration::from_secs(60))
    );
    assert!(!s.scanning);

    s.exporting = false;
    let now = t0 + 2 * hour + Duration::from_secs(61);
    assert_eq!(s.tick_schedule(now), Some(2 * hour), "re-armed");
    assert!(s.scanning, "Update databases was pressed");
    assert!(s.journal.to_text().contains("Scheduled sync"));
}

#[test]
fn a_database_for_a_chat_not_in_the_list_is_named_not_run() {
    // "Updated 11 databases" in a folder of 12 is a question with no answer on
    // screen unless the twelfth is named.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.scanning = true;
    s.apply(Event::Archived(Ok(Box::new(tgx_tg::engine::Archived {
        chats: vec![(9, "Gone".into())],
        unreadable: vec![],
        ..Default::default()
    }))));
    assert!(!s.scanning, "the button comes back");
    assert!(!s.exporting, "nothing to run");
    assert!(s.queue.is_empty());
    assert!(s.status.starts_with("No databases to update"));
    let log = s.journal.to_text();
    assert!(log.contains("Gone has a database here"), "got {log}");
}

#[test]
fn an_answer_about_another_destination_marks_nothing() {
    // The user changed the folder while the old one was being read; marking
    // chats from the old answer would claim they are in the new one.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.settings.output_dir = "new".into();
    let there = crate::list::OnDisk {
        database: true,
        ..Default::default()
    };
    s.apply(Event::OnDisk {
        output_dir: "old".into(),
        disk: [(1, there)].into(),
    });
    assert!(s.view.disk.is_empty());
    s.apply(Event::OnDisk {
        output_dir: "new".into(),
        disk: [(1, there)].into(),
    });
    assert!(s.view.disk[&1].database);
    // And the filter reads it on the next rebuild.
    s.view.presence = crate::list::Presence::NotExported;
    s.rebuild_rows();
    assert_eq!(s.visible_count, 0);
    assert_eq!(
        chats::on_disk_mark(true, false),
        Some("Database"),
        "a database alone"
    );
    assert_eq!(chats::on_disk_mark(false, false), None);
}

#[test]
fn recalling_a_selection_replaces_the_ticks_and_names_what_is_gone() {
    let mut s = shell_with(vec![
        chat(1, "a", None),
        chat(2, "b", None),
        chat(3, "c", None),
    ]);
    s.settings.saved_selections = vec![tgx_tg::config::SavedSelection {
        name: "orgs".into(),
        chats: vec![1, 2, 99],
    }];
    s.selected.insert(3);
    s.recall_selection("orgs");
    assert_eq!(s.selected, [1, 2].into(), "replaced, not added to");
    assert_eq!(s.selection_name, "orgs");
    assert!(s.status.contains("1 no longer in the list"), "{}", s.status);
}

// -- the viewer ----------------------------------------------------------------

/// A database the viewer can "open". The path does not exist, so the detail
/// request a selection sends fails offline and harmlessly; these tests are
/// about what the shell does with the answers it is handed.
fn db(chat_id: i64, title: &str) -> tgx_tg::viewer::Database {
    tgx_tg::viewer::Database {
        chat_id,
        title: title.into(),
        path: std::env::temp_dir()
            .join("tgx-viewer-shell-test-none")
            .join(format!("{title}.sqlite")),
    }
}

fn message(id: i64, deleted: bool, version: i64) -> tgx_tg::viewer::Row {
    tgx_tg::viewer::Row {
        id,
        topic_id: 1,
        date: format!("2025-12-18 16:{id:02}"),
        day: "2025-12-18".into(),
        time: format!("16:{id:02}"),
        from: "Ivana".into(),
        from_id: "user1".into(),
        colour: 1,
        initials: "I".into(),
        text: format!("message {id}"),
        preview: format!("message {id}"),
        media: None,
        media_detail: None,
        reply_to: None,
        forwarded: None,
        edited_at: None,
        deleted: deleted.then(|| "2025-12-20 10:00".into()),
        version,
        recovered: false,
        service: false,
    }
}

/// Rows as a load answers them, all in the one topic of a chat that is not
/// a forum.
fn loaded(rows: Vec<tgx_tg::viewer::Row>) -> tgx_tg::viewer::Loaded {
    tgx_tg::viewer::Loaded {
        rows,
        topics: vec![tgx_tg::viewer::Topic {
            id: 1,
            title: "General".into(),
        }],
    }
}

/// A shell with `db` open and its rows applied.
fn viewing(database: &tgx_tg::viewer::Database, rows: Vec<tgx_tg::viewer::Row>) -> Shell {
    let mut s = Shell::headless();
    s.show(View::Viewer);
    s.viewer_open(database.clone(), None);
    assert!(s.viewer.loading);
    s.apply(Event::ViewerLoaded {
        path: database.path.clone(),
        chat_id: database.chat_id,
        rows: Ok(loaded(rows)),
    });
    s
}

fn hit(id: i64) -> tgx_tg::search::Hit {
    tgx_tg::search::Hit {
        id,
        topic_id: 1,
        date: "2025-12-18T16:00:00".into(),
        deleted: false,
        in_earlier_version: false,
        excerpt: String::new(),
    }
}

#[test]
fn the_viewer_is_a_fourth_view_with_its_own_tab() {
    assert_eq!(View::ALL.len(), 4);
    assert_eq!(View::ALL[3], View::Viewer);
    assert_eq!(View::Viewer.label(), "Viewer");
    let mut s = Shell::headless();
    s.show(View::Viewer);
    assert_eq!(s.body, View::Viewer);
    // And a run starting does not pull the user out of it.
    s.suggest(View::Queue);
    assert_eq!(s.body, View::Viewer);
}

#[test]
fn a_loaded_chat_counts_its_markers_and_the_filter_narrows_it() {
    let d = db(7, "Dev");
    let mut s = viewing(
        &d,
        vec![
            message(1, false, 1),
            message(2, true, 1),
            message(3, false, 3),
        ],
    );
    assert!(!s.viewer.loading);
    assert_eq!(s.viewer.shown, vec![0, 1, 2]);
    assert_eq!(s.viewer.counts, (1, 1, 0));
    s.viewer_set_filter(viewer::Filter::Deleted);
    assert_eq!(s.viewer.shown, vec![1]);
    s.viewer_set_filter(viewer::Filter::Edited);
    assert_eq!(s.viewer.shown, vec![2]);
}

#[test]
fn a_forums_topics_can_be_picked_and_a_message_elsewhere_switches_to_its_topic() {
    let d = db(7, "Forum");
    let mut s = Shell::headless();
    s.show(View::Viewer);
    s.viewer_open(d.clone(), None);
    let mut rows: Vec<_> = (1..=6).map(|id| message(id, false, 1)).collect();
    for r in rows.iter_mut().filter(|r| r.id % 2 == 0) {
        r.topic_id = 5;
    }
    s.apply(Event::ViewerLoaded {
        path: d.path.clone(),
        chat_id: 7,
        rows: Ok(tgx_tg::viewer::Loaded {
            rows,
            topics: vec![
                tgx_tg::viewer::Topic {
                    id: 1,
                    title: "General".into(),
                },
                tgx_tg::viewer::Topic {
                    id: 5,
                    title: "Foto".into(),
                },
            ],
        }),
    });
    assert_eq!(s.viewer.shown.len(), 6, "every topic at first");
    s.viewer_set_topic(Some(5));
    assert_eq!(s.viewer.shown, vec![1, 3, 5], "only Foto's messages");
    // Each gets a day separator ahead of it, and the list re-measures.
    assert_eq!(s.viewer.items.len(), 4);
    assert!(s.viewer.heights.is_empty());

    // A hit in General, selected while Foto is shown, switches to General.
    s.viewer_select(3, true);
    assert_eq!(s.viewer.topic, Some(1));
    assert_eq!(s.viewer.selected, Some(3));
    assert_eq!(s.viewer.shown, vec![0, 2, 4]);

    // And opening another database forgets this one's topics.
    s.viewer_open(db(8, "Other"), None);
    assert!(s.viewer.topics.is_empty() && s.viewer.topic.is_none());
}

#[test]
fn rows_for_a_chat_no_longer_open_are_dropped() {
    // A slow load of the database the user just moved away from must not
    // replace the one on screen.
    let (a, b) = (db(1, "A"), db(2, "B"));
    let mut s = viewing(&a, vec![message(1, false, 1)]);
    s.viewer_open(b.clone(), None);
    s.apply(Event::ViewerLoaded {
        path: a.path.clone(),
        chat_id: 1,
        rows: Ok(loaded(vec![message(1, false, 1), message(2, false, 1)])),
    });
    assert!(s.viewer.loading, "still waiting for B");
    assert!(s.viewer.rows.is_empty());
}

#[test]
fn search_hits_in_this_chat_select_the_first_and_step_through_the_rest() {
    let d = db(7, "Dev");
    let rows = (1..=10).map(|id| message(id, false, 1)).collect();
    let mut s = viewing(&d, rows);
    // Filtered to Deleted, which none of the hits are: selecting one must put
    // the filter back rather than select a row nobody can see.
    s.viewer_set_filter(viewer::Filter::Deleted);
    s.viewer.query = "sastanak".into();
    s.viewer_search();
    let found = tgx_tg::search::Found {
        chats: vec![tgx_tg::search::ChatHits {
            chat_id: 7,
            title: "Dev".into(),
            path: d.path.clone(),
            hits: vec![hit(4), hit(9)],
        }],
        unreadable: vec![],
    };
    s.apply(Event::SearchHits {
        needle: "sastanak".into(),
        scope: Some(7),
        found: Ok(Box::new(found)),
    });
    assert_eq!(s.viewer.hits, vec![4, 9]);
    assert_eq!(s.viewer.selected, Some(4));
    assert_eq!(s.viewer.hit_at, Some(0));
    assert_eq!(s.viewer.filter, viewer::Filter::All);
    assert_eq!(s.viewer.scroll_to, Some(3), "the list jumps to row 4");

    s.viewer_step_hit(1);
    assert_eq!(s.viewer.selected, Some(9));
    s.viewer_step_hit(1);
    assert_eq!(s.viewer.selected, Some(4), "wraps");
    s.viewer_step_hit(-1);
    assert_eq!(s.viewer.selected, Some(9));
}

#[test]
fn an_answer_to_a_search_nobody_is_asking_any_more_is_dropped() {
    let d = db(7, "Dev");
    let mut s = viewing(&d, vec![message(1, false, 1)]);
    s.viewer.query = "new".into();
    s.viewer_search();
    s.apply(Event::SearchHits {
        needle: "old".into(),
        scope: Some(7),
        found: Ok(Box::default()),
    });
    assert!(
        s.viewer.asked.is_some(),
        "still waiting for the right answer"
    );
}

#[test]
fn a_hit_in_another_database_opens_that_chat_at_that_message() {
    let (a, b) = (db(1, "A"), db(2, "B"));
    let mut s = viewing(&a, vec![message(1, false, 1)]);
    s.viewer.everywhere = true;
    s.viewer.query = "ђорђе".into();
    s.viewer_search();
    let found = tgx_tg::search::Found {
        chats: vec![tgx_tg::search::ChatHits {
            chat_id: 2,
            title: "B".into(),
            path: b.path.clone(),
            hits: vec![hit(5), hit(8)],
        }],
        unreadable: vec![],
    };
    s.apply(Event::SearchHits {
        needle: "ђорђе".into(),
        scope: None,
        found: Ok(Box::new(found)),
    });
    assert!(s.viewer.showing_found);
    assert_eq!(s.viewer.found_rows, vec![(0, 0), (0, 1)]);

    s.viewer_open_hit(0, 1);
    assert!(!s.viewer.showing_found);
    assert_eq!(s.viewer.open.as_ref().map(|d| d.chat_id), Some(2));
    assert!(s.viewer.loading);
    s.apply(Event::ViewerLoaded {
        path: b.path.clone(),
        chat_id: 2,
        rows: Ok(loaded((1..=9).map(|id| message(id, false, 1)).collect())),
    });
    assert_eq!(s.viewer.selected, Some(8), "the clicked hit, once loaded");
    assert_eq!(s.viewer.scroll_to, Some(7));
    assert_eq!(
        s.viewer.hits,
        vec![5, 8],
        "and its chat's hits for Prev / Next"
    );
    assert_eq!(s.viewer.hit_at, Some(1));
}

#[test]
fn a_detail_for_a_message_no_longer_selected_is_dropped() {
    let d = db(7, "Dev");
    let mut s = viewing(&d, vec![message(1, false, 1), message(2, false, 1)]);
    s.viewer_select(2, false);
    let detail = |id| tgx_tg::viewer::Detail {
        id,
        date: String::new(),
        from: String::new(),
        text: "x".into(),
        reply_to: None,
        edited: None,
        deleted: None,
        recovered: false,
        action: None,
        versions: vec![],
        media: vec![],
    };
    s.apply(Event::ViewerDetail {
        chat_id: 7,
        id: 1,
        detail: Ok(Some(Box::new(detail(1)))),
    });
    assert!(s.viewer.detail.is_none());
    s.apply(Event::ViewerDetail {
        chat_id: 7,
        id: 2,
        detail: Ok(Some(Box::new(detail(2)))),
    });
    assert_eq!(s.viewer.detail.as_ref().map(|d| d.id), Some(2));
}

#[test]
fn update_databases_does_not_join_a_classic_run() {
    // That run has its own copy of the settings; adding to it would export the
    // chats as folders beside the databases the user asked to update.
    let mut s = shell_with(vec![chat(1, "a", None)]);
    s.exporting = true;
    s.settings.export_db = false;
    s.apply(Event::Archived(Ok(Box::new(tgx_tg::engine::Archived {
        chats: vec![(1, "a".into())],
        unreadable: vec![],
        ..Default::default()
    }))));
    assert!(s.queue.is_empty());
    assert!(!s.settings.export_db, "the mode is left alone");
}

//! Running an export: resolving a chat's topics, driving the engine, and
//! reporting what the run actually produced.
//!
//! **The queue owns what a run did.** Nothing here composes its own summary,
//! because the second writer always wins a race you did not know you had: a
//! worker's cheerful "Exported 3 of 3 chats" once landed on top of "Stopped" a
//! moment after the user pressed it.
//!
//! Topic discovery is separated from the export itself because a chat that
//! refuses its topic list must still export as one folder rather than ending
//! the queue.

use super::*;

/// What discovering one chat's topics decided.
pub(super) enum TopicsResolution {
    /// Genuinely split, with the topics Telegram returned.
    Split(Vec<dialogs::Topic>),
    /// A real refusal, not a rate limit: there is no shape to preserve, so the
    /// chat exports as one folder, the same answer a chat that was never a
    /// forum gets.
    Unsplit,
    /// Still rate-limited after the one retry [`resolve_topics`] gives it, or
    /// Telegram still failing on its side after `client::Patient`'s three.
    /// **Not** the same answer as [`Unsplit`](Self::Unsplit) — a forum stays
    /// a forum, it just cannot be exported this run.
    TryLater,
    /// Cancelled while waiting out the rate limit.
    Cancelled,
}

/// Resolve a forum's topics, waiting out one rate limit before giving up on
/// it — the same patience `dialogs::count_all` gives a single chat.
///
/// **Splitting by topic is this app's entire reason to exist.** Before this,
/// `export` caught *any* error from `list_topics`, `Transient` included, and
/// fell back to `vec![Topic::general()]` — so a `FloodWait` during discovery
/// silently turned a forum into a single folder, the one shape this app was
/// built not to produce. A genuine refusal has no shape left to preserve and
/// still degrades; a rate limit does not get to make that call.
///
/// `fetch` stands in for `dialogs::list_topics`, which needs a live socket to
/// answer at all — this way the distinction above can be driven with a canned
/// [`EnrichError`] in a test instead.
pub(super) async fn resolve_topics<F, Fut>(
    chat_title: &str,
    chat_id: i64,
    cancel: &Cancel,
    tx: &Events,
    mut fetch: F,
) -> TopicsResolution
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<dialogs::Topic>, EnrichError>>,
{
    let mut retried = false;
    loop {
        match fetch().await {
            Ok(t) => return TopicsResolution::Split(t),
            Err(e) if e.is_transient() && !retried => {
                retried = true;
                let secs = e.retry_after().unwrap_or_default();
                let _ = tx.send(Event::FloodWait(secs.as_secs()));
                // Sliced and cancellable: a click on Stop during the wait must
                // not be held for the whole of Telegram's two minutes.
                tgx_tg::engine::sleep_in_slices_until(secs, cancel).await;
                if cancel.is_cancelled() {
                    return TopicsResolution::Cancelled;
                }
            }
            Err(e) if e.is_transient() => {
                // The retry above already spent this chat's share of the
                // queue's patience; a second wait would hold up every chat
                // behind it for the sake of one Telegram is still throttling.
                let _ = tx.send(Event::ChatFailed {
                    chat_id,
                    message: "rate limited while listing topics — retry this \
                              chat later rather than lose its topic split"
                        .into(),
                });
                return TopicsResolution::TryLater;
            }
            // Telegram failed on its side, and `client::Patient` has already
            // asked three more times. Falling through to the arm below would
            // export the forum as one folder — item 11's bug again, for a
            // different temporary cause.
            Err(e) if e.is_unavailable() => {
                let _ = tx.send(Event::ChatFailed {
                    chat_id,
                    message: format!(
                        "Telegram failed on its side while listing topics ({e}) — \
                         retry this chat later rather than lose its topic split"
                    ),
                });
                return TopicsResolution::TryLater;
            }
            Err(e) => {
                let _ = tx.send(Event::Warn(format!(
                    "{chat_title}: could not list topics ({e}); exporting as one folder"
                )));
                return TopicsResolution::Unsplit;
            }
        }
    }
}

/// Export every queued chat, in queue order.
///
/// **Per-chat tallies live on the result, never on a shared counter** — see
/// `tgx_tg::engine::ExportResult`. This function keeps nothing across chats
/// except the resolved peers.
///
/// **The work list is shared, not a snapshot.** It used to be a `Vec` handed
/// over at spawn time, which made a run a fixed set of chats: adding one meant
/// stopping and starting again, and losing whatever was in flight. Popping from
/// [`Pending`] instead means the window can push while this loop runs, and a ×
/// on a queued row removes a chat this loop then never sees.
pub async fn export(settings: Settings, pending: Pending, cancel: Cancel, tx: Events) {
    use tgx_tg::engine::{ChatExporter, Progress};

    // A panic in the engine ends the queue the way a failed start does: the
    // interrupted chat is marked paused, not left reading "Exporting".
    on_panic(|why| Activity::Export.ending(why));
    let session = match Session::connect(&settings).await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(Event::Failed {
                activity: Activity::Export,
                message: e.to_string(),
            });
            let _ = tx.send(Event::Finished { stopped: true });
            return;
        }
    };
    if !ready(&session, &tx, Activity::Export).await {
        let _ = tx.send(Event::Finished { stopped: true });
        return;
    }

    // **One dialog sweep for the whole queue**, before any message is fetched.
    // Resolving each chat inside the loop paged the entire dialog list once per
    // chat — twenty full sweeps for a twenty-chat queue, to answer twenty
    // questions that one response already contains, which is exactly the
    // pattern that earns a flood wait before the export has written a byte.
    //
    // Read off the list rather than from an argument: what is waiting *now* is
    // what this sweep has to cover, and anything added later is picked up by
    // the one below.
    let ids: Vec<i64> = pending.snapshot().iter().map(|c| c.id).collect();
    let mut peers = match dialogs::peer_refs_for(&session.client, &ids, &cancel).await {
        Ok(p) => p,
        Err(e) => {
            // A transport failure here is not "these chats are gone" — that
            // distinction is the whole reason `peer_refs_for` returns a
            // `Result`, and reporting it per chat would tell the user to go
            // looking for conversations they still have.
            let _ = tx.send(Event::Failed {
                activity: Activity::Export,
                message: format!("listing chats: {e}"),
            });
            let _ = tx.send(Event::Finished { stopped: true });
            return;
        }
    };

    // **Once for the whole queue**, like the one Telegram connection: it holds
    // the name book and the peer cache, which mean the same thing in every
    // chat. The database no longer opens here — there is one file per chat and
    // `run` opens it, so a database that will not open fails that chat and the
    // queue carries on.
    let mut exporter = ChatExporter::new(&session.client, &settings, session.session());

    // **The database each chat already has, found by the id inside it**, as
    // the command line's `export --db` and `sync` do. Without it the engine
    // opens the file the chat's *current* title names, so a group renamed
    // since the last sync started a fresh database under every new name. An
    // unreadable folder is "none yet": the title then names the file, as for
    // a chat never synced.
    let archived = if settings.export_db {
        tgx_tg::engine::archived_chats(std::path::Path::new(&settings.output_dir))
            .await
            .map(|a| a.files)
            .unwrap_or_default()
    } else {
        Default::default()
    };

    // **Popped, not iterated.** The list is the window's too, so its length is
    // not known here and can grow between two turns of this loop.
    while let Some(chat) = pending.take_next() {
        if cancel.is_cancelled() {
            break;
        }
        let chat = &chat;
        let _ = tx.send(Event::ChatStarted {
            chat_id: chat.id,
            title: chat.title.clone(),
        });

        let peer = match peers.get(&chat.id).copied() {
            Some(p) => p,
            None => {
                // Queued after the run began, so the sweep above ran before
                // anyone asked for it. Swept for **everything now waiting**
                // rather than for this chat alone: a queue added to five times
                // would otherwise pay five passes over the whole dialog list,
                // which is the cost this sweep exists to avoid.
                let mut later: Vec<i64> = pending.snapshot().iter().map(|c| c.id).collect();
                later.push(chat.id);
                match dialogs::peer_refs_for(&session.client, &later, &cancel).await {
                    Ok(found) => peers.extend(found),
                    Err(e) => {
                        // One chat's failure, not the queue's: the first sweep
                        // succeeded, so the connection worked a moment ago and
                        // the chats behind this one deserve their turn.
                        let _ = tx.send(Event::ChatFailed {
                            chat_id: chat.id,
                            message: format!("listing chats: {e}"),
                        });
                        continue;
                    }
                }
                match peers.get(&chat.id).copied() {
                    Some(p) => p,
                    None => {
                        let _ = tx.send(Event::ChatFailed {
                            chat_id: chat.id,
                            message: "no longer in the dialog list".into(),
                        });
                        continue;
                    }
                }
            }
        };

        // Whether this chat is genuinely split into topic folders. Held rather
        // than recomputed, because it decides both what the engine does and
        // what the queue is allowed to claim afterwards — and a topic count
        // reported for a chat that was never split is a wrong number that
        // reads as a right one.
        let mut split = chat.is_forum && settings.split_topics;
        let topics = if split {
            match resolve_topics(&chat.title, chat.id, &cancel, &tx, || {
                dialogs::list_topics(&session.client, peer)
            })
            .await
            {
                TopicsResolution::Split(t) => t,
                // A genuine refusal, not a rate limit: there is no shape to
                // preserve, so this one chat degrades to a single folder —
                // the same answer a chat that was never a forum gets.
                TopicsResolution::Unsplit => {
                    split = false;
                    vec![dialogs::Topic::general()]
                }
                // Still rate-limited after the one retry, or the wait for it
                // was cancelled. Neither is "this chat has no topics", so
                // neither may fall into the branch above and export a forum
                // as one folder — `resolve_topics` has already said which one
                // happened.
                TopicsResolution::TryLater => continue,
                TopicsResolution::Cancelled => break,
            }
        } else {
            vec![dialogs::Topic::general()]
        };
        if split {
            let _ = tx.send(Event::ChatTopics {
                chat_id: chat.id,
                topics: topics.len(),
            });
        }

        // **No folder when nothing goes into one.** With only the Database
        // format ticked there is nothing to write, and reserving a directory
        // anyway would leave an empty `Dev Team (7)` behind on every sync.
        let root = if settings.writes_folders() {
            match tgx_tg::engine::unique_dir(
                std::path::Path::new(&settings.output_dir),
                &chat.title,
            ) {
                Ok(r) => {
                    let _ = tx.send(Event::Log(format!(
                        "{}: writing to {}",
                        chat.title,
                        r.display()
                    )));
                    Some(r)
                }
                Err(e) => {
                    // The destination is free text and may be unwritable,
                    // disconnected or invalid. That ends the whole queue, not
                    // one chat: every later chat would fail the same way.
                    let _ = tx.send(Event::Failed {
                        activity: Activity::Export,
                        message: format!("cannot write into {}: {e}", settings.output_dir),
                    });
                    let _ = tx.send(Event::Finished { stopped: true });
                    return;
                }
            }
        } else {
            let _ = tx.send(Event::Log(format!(
                "{}: into the database only — no folder",
                chat.title
            )));
            None
        };

        let chat_id = chat.id;
        let tx2 = tx.clone();
        let mut on_progress = move |p: Progress| match p {
            Progress::Messages { done, total, .. } => {
                let _ = tx2.send(Event::Progress {
                    chat_id,
                    done,
                    total,
                });
            }
            Progress::Total { total, .. } => {
                let _ = tx2.send(Event::ChatTotal { chat_id, total });
            }
            Progress::FloodWait { seconds } => {
                let _ = tx2.send(Event::FloodWait(seconds));
            }
            Progress::Stage(stage) => {
                let _ = tx2.send(Event::Stage { chat_id, stage });
            }
            Progress::Topic { title, messages } => {
                log::info!("{title}: {messages}");
                let _ = tx2.send(Event::Log(format!("  {title}: {messages}")));
            }
            Progress::Log(msg) => {
                // **Both channels.** The window's transcript is a 2,000-line
                // ring that dies with the process; `tgx.log` survives it and is
                // the only thing left to read when someone asks the next day
                // why an export was short.
                log::info!("{msg}");
                let _ = tx2.send(Event::Log(format!("  {msg}")));
            }
            // Deliberately not sent to the window: one chat of six thousand
            // messages would flush every other line out of the ring, including
            // the INCOMPLETE warning the ring exists to preserve.
            Progress::Detail(msg) => log::debug!("{msg}"),
        };

        if let Some(path) = archived.get(&chat.id) {
            exporter.sync_into(path.clone());
        }
        match exporter
            .run(
                chat,
                peer,
                &topics,
                root.as_deref(),
                &mut on_progress,
                &cancel,
            )
            .await
        {
            Ok(result) => {
                let _ = tx.send(chat_done(chat_id, &result, split));
                report_result(&tx, &chat.title, &result, split);
            }
            Err(tgx_tg::ExportError::Cancelled) => {
                // Not a failure. The user asked it to stop, and whatever was
                // written is closed and valid — `close_all` ran on the way out.
                let _ = tx.send(Event::Log(format!("{}: stopped", chat.title)));
                break;
            }
            Err(e) => {
                let _ = tx.send(Event::ChatFailed {
                    chat_id,
                    message: e.to_string(),
                });
            }
        }
    }

    let _ = tx.send(Event::Finished {
        stopped: cancel.is_cancelled(),
    });
}

/// The event a finished chat sends: what the queue's row and the chat list
/// learn from the run.
///
/// Pulled out of the loop so a hand-built `ExportResult` can drive it. This is
/// where a short-by-design run stops being a measurement, and nothing tested
/// that before: a swapped condition here would let a 500-message sync
/// overwrite the list's 6,643 with the suite still green.
fn chat_done(chat_id: i64, result: &tgx_tg::engine::ExportResult, split: bool) -> Event {
    Event::ChatDone {
        chat_id,
        messages: result.messages,
        // **A windowed run reports no expected count and no measured one.** It
        // read the newest few hundred messages on purpose; handing that up
        // would make the row read "500 of 6,643" with an 8% bar, and
        // `set_count` would then overwrite the chat list's size with 500. See
        // `ExportResult::complete`, which draws the same distinction for the
        // INCOMPLETE warning. A date-ranged export is short on purpose in the
        // same way.
        expected: if result.short_by_design() {
            0
        } else {
            result.expected
        },
        messages_measured: !result.short_by_design(),
        // `result.topics` counts output folders, and an unsplit chat has
        // exactly one. Only a chat that really was split has a topic count to
        // report.
        topics: split.then_some(result.topics),
        media_downloaded: result.media_downloaded,
        media_failed: result.media_failed,
        root: result.root.clone(),
        database: result
            .database
            .clone()
            .map(std::path::PathBuf::into_boxed_path),
    }
}

/// Everything a finished chat has to admit to.
///
/// Each of these was counted and then never shown at some point in the
/// original, which is the same as not having counted it.
fn report_result(tx: &Events, title: &str, result: &tgx_tg::engine::ExportResult, split: bool) {
    let mb = result.bytes_downloaded as f64 / (1024.0 * 1024.0);
    let mut line = format!("{title}: {} messages", result.messages);
    // Only a chat that was split has topic folders. `result.topics` counts
    // output folders, and an unsplit chat has one — which read as
    // "1 topic folders" on every private chat.
    //
    // "Folders" only where there are some: a database-only run writes none,
    // and every chat of a twelve-chat queue still said "17 topic folders".
    if split && result.topics > 0 {
        let folders = result.root.is_some();
        line.push_str(&format!(
            ", {} {}",
            result.topics,
            if folders { "topic folders" } else { "topics" }
        ));
        if result.empty_topics > 0 {
            line.push_str(&format!(
                " ({} empty{})",
                result.empty_topics,
                if folders { " skipped" } else { "" }
            ));
        }
    }
    line.push_str(&format!(", {} files ({mb:.1} MB)", result.media_downloaded));
    let _ = tx.send(Event::Log(line));

    // The database gets a line of its own rather than four more clauses on the
    // one above: the two are different accounts of the run. The first is what
    // this export produced; this is what the archive holds now.
    if result.db_archived > 0 || result.db_new > 0 {
        let _ = tx.send(Event::Log(format!(
            "{title}: database +{} new, {} changed, {} deleted on Telegram (kept), \
             {} files stored — {} archived",
            result.db_new,
            result.db_changed,
            result.db_deleted,
            result.db_blobs,
            result.db_archived
        )));
    }

    // **A short export must not read like a complete one.** A crash at message
    // 5,609 of 6,600 produced a cheerful summary and a thousand missing
    // messages, and nothing on screen distinguished it from a whole export.
    if !result.complete() {
        let _ = tx.send(Event::Warn(format!(
            "{title}: INCOMPLETE — Telegram counted {}, {} came through",
            result.expected, result.messages
        )));
    }
    // `media_missing`, not `media_failed`: this line names a number and points
    // at a file, so it has to be the number of lines in that file. One failed
    // job takes its thumbnail and its preview with it, and the run reported 21
    // over a missing_media.txt that listed 42.
    //
    // A database-only run has no folder to hold missing_media.txt, and this
    // sent the user looking for one anyway. The paths are in tgx.log then, and
    // only a full re-read reaches back far enough to ask for them again.
    if result.media_missing > 0 {
        let _ = tx.send(Event::Warn(format!(
            "{title}: {} files could not be fetched — {}",
            result.media_missing,
            if result.root.is_some() {
                "see missing_media.txt"
            } else {
                "each one is named in tgx.log, and \"Check the whole history\" \
                 asks for them again"
            }
        )));
    }
    if result.members > 0 && !result.members_complete {
        // A truncated roster that looks complete is worse than no roster:
        // wrong data that reads as right.
        let _ = tx.send(Event::Warn(format!(
            "{title}: the member list is INCOMPLETE — see participants.json"
        )));
    }
    if result.enrich_deferred > 0 {
        let n = result.enrich_deferred;
        let (lookups, were) = if n == 1 {
            ("lookup", "was")
        } else {
            ("lookups", "were")
        };
        let _ = tx.send(Event::Warn(format!(
            "{title}: {n} extra {lookups} {were} lost to rate limits or Telegram \
             server errors — reaction \
             names, poll results or custom emoji may be missing. Re-exporting \
             this chat later will fill them in."
        )));
    }
    if !result.degraded.is_empty() {
        let _ = tx.send(Event::Log(format!(
            "{title}: Telegram data this build has no mapping for, written as \
             text: {}",
            result.degraded.join(", ")
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tgx_tg::engine::ExportResult;

    /// The warnings `report_result` sends for `result`, as plain lines.
    fn warnings(result: &ExportResult, split: bool) -> Vec<String> {
        let (raw, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = Events::detached(raw);
        report_result(&tx, "Dev Team", result, split);
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            if let Event::Warn(m) = e {
                out.push(m);
            }
        }
        out
    }

    /// `(expected, messages_measured, topics)` off the `ChatDone`.
    fn done(result: &ExportResult, split: bool) -> (i64, bool, Option<usize>) {
        match chat_done(7, result, split) {
            Event::ChatDone {
                chat_id: 7,
                expected,
                messages_measured,
                topics,
                ..
            } => (expected, messages_measured, topics),
            other => panic!("not a ChatDone: {other:?}"),
        }
    }

    fn whole() -> ExportResult {
        ExportResult {
            messages: 6_643,
            expected: 6_643,
            topics: 1,
            reached_end: true,
            root: Some("Exports/Dev Team".into()),
            ..ExportResult::default()
        }
    }

    #[test]
    fn a_whole_export_is_a_measurement_and_admits_nothing() {
        assert_eq!(done(&whole(), false), (6_643, true, None));
        assert!(warnings(&whole(), false).is_empty());
    }

    #[test]
    fn a_short_export_says_incomplete() {
        let short = ExportResult {
            messages: 5_609,
            ..whole()
        };
        // Still a measurement: the run walked the whole history and this is
        // what came through, short or not.
        assert_eq!(done(&short, false), (6_643, true, None));
        let warned = warnings(&short, false);
        assert!(
            warned
                .iter()
                .any(|m| m.contains("INCOMPLETE") && m.contains("6643")),
            "got {warned:?}"
        );
    }

    #[test]
    fn a_windowed_sync_is_not_the_size_of_the_chat() {
        // The newest 500 of 6,643, on purpose. Handed up as a measurement it
        // would overwrite the list's count with 500.
        let windowed = ExportResult {
            messages: 500,
            windowed: true,
            root: None,
            database: Some("Exports/Dev Team/Dev Team.sqlite".into()),
            ..whole()
        };
        assert_eq!(done(&windowed, false), (0, false, None));
        assert!(warnings(&windowed, false).is_empty(), "short by design");

        // A window that did not reach the end of what it set out to read is
        // still incomplete — `reached_end`, not the count, is the test.
        let cut = ExportResult {
            reached_end: false,
            ..windowed
        };
        assert!(warnings(&cut, false)
            .iter()
            .any(|m| m.contains("INCOMPLETE")));
    }

    #[test]
    fn a_date_ranged_export_is_short_on_purpose_too() {
        let ranged = ExportResult {
            messages: 120,
            ranged: true,
            ..whole()
        };
        assert_eq!(done(&ranged, false), (0, false, None));
        assert!(warnings(&ranged, false).is_empty());
    }

    #[test]
    fn only_a_split_chat_reports_topics() {
        let forum = ExportResult {
            topics: 4,
            ..whole()
        };
        assert_eq!(done(&forum, true).2, Some(4));
        assert_eq!(done(&forum, false).2, None);
    }

    #[test]
    fn missing_media_points_at_the_file_that_lists_it() {
        let folder = ExportResult {
            media_missing: 42,
            media_failed: 21,
            ..whole()
        };
        let warned = warnings(&folder, false);
        assert!(
            warned
                .iter()
                .any(|m| m.contains("42 files") && m.contains("missing_media.txt")),
            "got {warned:?}"
        );

        // A database-only run has no folder to hold that file.
        let database = ExportResult {
            root: None,
            ..folder
        };
        let warned = warnings(&database, false);
        assert!(
            warned
                .iter()
                .any(|m| m.contains("42 files") && m.contains("tgx.log")),
            "got {warned:?}"
        );
        assert!(!warned.iter().any(|m| m.contains("missing_media.txt")));
    }

    #[test]
    fn a_partial_roster_says_so_and_a_whole_one_does_not() {
        let partial = ExportResult {
            members: 200,
            members_complete: false,
            ..whole()
        };
        assert!(warnings(&partial, false)
            .iter()
            .any(|m| m.contains("member list is INCOMPLETE")));
        let complete = ExportResult {
            members_complete: true,
            ..partial
        };
        assert!(warnings(&complete, false).is_empty());
    }
}

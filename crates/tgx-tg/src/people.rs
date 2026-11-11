//! Who is in a set of chats, asked of Telegram, with nothing exported.
//!
//! Two passes, and the caller picks:
//!
//! - **members**: each chat's member list, the request an export's roster
//!   makes. Seconds. No message counts, and it names people who never wrote.
//!   Telegram hides the list of a broadcast channel, and of some large groups,
//!   from anyone not an admin; such a chat is said, not passed over.
//! - **messages**: every message of each chat, read the way an export reads
//!   them but written nowhere and with no media fetched. Exact, people who left
//!   included, and as slow as an export's text pass.
//!
//! Each chat comes back on its own, keyed by the typed peer key an export
//! writes as `from_id` (`user123`), never by name; the caller merges them, so
//! one person is one row however many chats they are in.

use std::collections::{BTreeMap, HashMap};

use grammers_client::Client;
use grammers_tl_types as tl;
use serde_json::Value;

use crate::cancel::Cancel;
use crate::config::Settings;
use crate::convert::{peer_key, NameBook};
use crate::engine::{sleep_in_slices_until, MAX_STALLED_WAITS, UNAVAILABLE_PAUSE};
use crate::enrich::{fetch_participants, Enrichment};
use crate::error::{classify, EnrichError};

/// One chat, read whole.
#[derive(Debug, Clone, Default)]
pub struct Counted {
    pub id: i64,
    pub title: String,
    /// Peer key → (name, messages). Messages are 0 after a members pass.
    pub people: BTreeMap<String, (String, usize)>,
}

/// What a pass found.
#[derive(Debug, Default)]
pub struct Census {
    /// Every chat read whole. **Only those**: a member list Telegram cut
    /// short or a read that was stopped would be a wrong count that looks
    /// right, and a caller that keeps the list would keep it.
    pub chats: Vec<Counted>,
    /// One line per chat that was not read whole, saying why.
    pub short: Vec<String>,
    /// Stop was pressed.
    pub stopped: bool,
}

/// Count everybody in `chats` (id, title). `progress` hears a line now and
/// then; it never carries a message's text, only counts and titles.
pub async fn census(
    client: &Client,
    chats: &[(i64, String)],
    settings: &Settings,
    messages: bool,
    cancel: &Cancel,
    progress: &mut (dyn FnMut(String) + Send),
) -> Result<Census, EnrichError> {
    let ids: Vec<i64> = chats.iter().map(|(id, _)| *id).collect();
    let peers = crate::dialogs::peer_refs_for(client, &ids, cancel).await?;
    let mut out = Census::default();
    let total = chats.len();
    for (index, (id, title)) in chats.iter().enumerate() {
        if cancel.is_cancelled() {
            out.stopped = true;
            break;
        }
        progress(format!("{} of {total}: {title}", index + 1));
        let Some(peer) = peers.get(id).copied() else {
            out.short
                .push(format!("{title}: no longer in the chat list"));
            continue;
        };
        let here = if messages {
            senders(client, peer, settings.own_names, cancel, progress).await
        } else {
            members(client, peer, settings, cancel, progress).await
        };
        match here {
            Ok((people, None)) => out.chats.push(Counted {
                id: *id,
                title: title.clone(),
                people: people.into_iter().collect(),
            }),
            Ok((_, Some(why))) => out.short.push(format!("{title}: {why}; not updated")),
            Err(e) => out.short.push(format!("{title}: {e}; not updated")),
        }
    }
    out.stopped |= cancel.is_cancelled();
    Ok(out)
}

/// One chat's people: key → (name, messages).
type Here = HashMap<String, (String, usize)>;

/// The member list, whole: the user's member limit is for exports, and a
/// count capped at it would be a wrong number that looks right.
async fn members(
    client: &Client,
    peer: crate::dialogs::ChatRef,
    settings: &Settings,
    cancel: &Cancel,
    progress: &mut (dyn FnMut(String) + Send),
) -> Result<(Here, Option<String>), EnrichError> {
    let mut whole = settings.clone();
    whole.member_roster = true;
    whole.member_limit = 0;
    let roster = fetch_participants(
        client,
        peer,
        &whole,
        &mut Enrichment::default(),
        cancel,
        |s| progress(format!("Telegram asked to wait {s}s")),
    )
    .await;
    let here: Here = roster
        .members
        .iter()
        .filter_map(|m| {
            let key = m.get("id").and_then(Value::as_str)?;
            let name = m.get("name").and_then(Value::as_str).unwrap_or("");
            Some((key.to_string(), (name.to_string(), 0)))
        })
        .collect();
    let why = (!roster.complete).then(|| {
        if here.is_empty() {
            "Telegram did not give the member list (often admins only)".to_string()
        } else {
            format!("Telegram gave only {} members", here.len())
        }
    });
    Ok((here, why))
}

/// Every message, newest first, counted by sender. Service entries (joins,
/// pins) are not somebody talking and are not counted. Resumes from the last
/// message read on a rate limit or a Telegram fault, as the export does.
async fn senders(
    client: &Client,
    peer: crate::dialogs::ChatRef,
    own_names: bool,
    cancel: &Cancel,
    progress: &mut (dyn FnMut(String) + Send),
) -> Result<(Here, Option<String>), EnrichError> {
    let mut names = NameBook {
        own_names,
        ..NameBook::default()
    };
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut offset_id = 0i32;
    let mut stalled = 0u32;
    let mut read = 0usize;
    'resume: loop {
        let mut iter = client.iter_messages(peer).offset_id(offset_id);
        loop {
            if cancel.is_cancelled() {
                break 'resume;
            }
            match iter.next().await {
                Ok(Some(msg)) => {
                    stalled = 0;
                    offset_id = msg.id();
                    read += 1;
                    if read.is_multiple_of(5_000) {
                        progress(format!("{read} messages read"));
                    }
                    let tl::enums::Message::Message(raw) = &msg.raw else {
                        continue;
                    };
                    // The sender comes with the page; a channel's post is the
                    // channel's, named by its title.
                    match msg.sender() {
                        Some(grammers_client::peer::Peer::User(u)) => {
                            if let tl::enums::User::User(user) = &u.raw {
                                names.learn_user(user);
                            }
                        }
                        Some(grammers_client::peer::Peer::Channel(c)) => names.learn_chat_title(
                            tgx_format::peer::PeerKey::channel(c.raw.id),
                            c.title(),
                        ),
                        _ => {}
                    }
                    if let Some(from) = crate::convert::sender(raw) {
                        *counts.entry(peer_key(from).to_string()).or_default() += 1;
                    }
                }
                Ok(None) => break 'resume,
                Err(e) => {
                    let pause = match classify(&e) {
                        EnrichError::Transient(d) => d,
                        EnrichError::Unavailable(_) => UNAVAILABLE_PAUSE,
                        other => return Err(other),
                    };
                    stalled += 1;
                    if stalled >= MAX_STALLED_WAITS {
                        let why = format!("stopped after {read} messages: Telegram kept refusing");
                        return Ok((with_names(counts, &names), Some(why)));
                    }
                    progress(format!("waiting {}s for Telegram", pause.as_secs()));
                    sleep_in_slices_until(pause, cancel).await;
                    continue 'resume;
                }
            }
        }
    }
    let why = cancel
        .is_cancelled()
        .then(|| format!("stopped after {read} messages"));
    Ok((with_names(counts, &names), why))
}

fn with_names(counts: HashMap<String, usize>, names: &NameBook) -> Here {
    counts
        .into_iter()
        .map(|(key, n)| {
            let name = names.get(&key).to_string();
            (key, (name, n))
        })
        .collect()
}

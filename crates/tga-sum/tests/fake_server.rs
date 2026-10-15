//! The whole pipeline against a stand-in llama-server: a `TcpListener` on
//! loopback that answers `/health`, `/props`, `/tokenize` and chat
//! completions — streamed in chunks, as the real one does — with canned text
//! built from the ids it was sent. No GPU, no model, no chat.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::NaiveDate;
use serde_json::{json, Value};
use tga_notes::Row;
use tga_sum::ocr::{read_all, Photo};
use tga_sum::{
    answer, ground, summarise, Cache, Client, Error, Event, Lang, Length, Request, Turn,
};

/// An id no message has. Every canned answer cites it, so every test also
/// checks that it never reaches the summary.
const STRANGER: i64 = 999_999;

#[derive(Default)]
struct Script {
    n_ctx: usize,
    /// Answer this many notes calls with something that is not JSON first.
    garbage: AtomicUsize,
    /// A question's pick chooses no note.
    pick_none: AtomicBool,
    /// Every chat request, as sent.
    seen: Mutex<Vec<Value>>,
}

struct Fake {
    port: u16,
    script: Arc<Script>,
}

impl Fake {
    fn start(n_ctx: usize) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let script = Arc::new(Script {
            n_ctx,
            ..Default::default()
        });
        let s = script.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let s = s.clone();
                std::thread::spawn(move || serve(conn, &s));
            }
        });
        Fake { port, script }
    }

    fn client(&self) -> Client {
        Client::new(self.port)
    }

    fn chats(&self) -> Vec<Value> {
        self.script.seen.lock().unwrap().clone()
    }

    fn notes_calls(&self) -> usize {
        self.chats()
            .iter()
            .filter(|b| b.get("response_format").is_some())
            .count()
    }
}

fn serve(conn: TcpStream, s: &Script) {
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let mut length = 0usize;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).unwrap();
        if h.trim().is_empty() {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            length = v.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
    let mut out = conn;
    match path.as_str() {
        "/health" => plain(&mut out, &json!({"status": "ok"})),
        "/props" => plain(
            &mut out,
            &json!({"model_path": "J:\\models\\gemma-test.gguf",
                     "default_generation_settings": {"n_ctx": s.n_ctx}}),
        ),
        "/tokenize" => {
            let n = body["content"].as_str().unwrap_or("").len().div_ceil(4);
            plain(&mut out, &json!({"tokens": vec![0; n]}))
        }
        "/v1/chat/completions"
            if body
                .pointer("/messages/0/content")
                .is_some_and(Value::is_array) =>
        {
            // An image to read, as ReVision's OCR is asked: a loop to cut and
            // a Russian letter to make Serbian.
            s.seen.lock().unwrap().push(body.clone());
            plain(
                &mut out,
                &json!({"choices": [{"message": {"content":
                    "ПЛАКАТ: ЛЬУДИ ИСПРЕД РЕКТОРАТА\nu petak u podne u podne u podne u podne"}}]}),
            );
        }
        "/v1/chat/completions" => {
            s.seen.lock().unwrap().push(body.clone());
            // Every message of the conversation: a question's days are in
            // its system prompt.
            let all: String = body["messages"]
                .as_array()
                .map(|m| {
                    m.iter()
                        .filter_map(|x| x["content"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let ids = ids_in(&all);
            if body.pointer("/response_format/json_schema/name") == Some(&json!("picked")) {
                // The first note, and one that does not exist.
                let picked = if s.pick_none.load(Ordering::SeqCst) {
                    json!({"notes": []})
                } else {
                    json!({"notes": [1, 999]})
                };
                plain(
                    &mut out,
                    &json!({"choices": [{"message": {"content": picked.to_string()}}]}),
                );
            } else if body.get("response_format").is_some() {
                let content = if s
                    .garbage
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_ok()
                {
                    "{\"notes\": [ {\"day\": \"2024-1".to_string()
                } else {
                    json!({"notes": [{"day": "2024-12-02", "weight": "major",
                                      "what": "Something was decided.",
                                      "ids": [ids[0], STRANGER]}]})
                    .to_string()
                };
                plain(
                    &mut out,
                    &json!({"choices": [{"message": {"content": content}}],
                            "timings": {"prompt_n": 100, "predicted_n": 20,
                                        "prompt_per_second": 1000.0, "predicted_per_second": 50.0}}),
                );
            } else {
                let text = format!("\nIt was decided [#{}] and denied [#{STRANGER}].", ids[0]);
                streamed(&mut out, &text);
            }
        }
        _ => {
            let _ = out.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        }
    }
}

/// Every `#123` in the text, in order — a message line's id or a note's.
fn ids_in(text: &str) -> Vec<i64> {
    let mut out = Vec::new();
    for (i, _) in text.match_indices('#') {
        let digits: String = text[i + 1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if let Ok(id) = digits.parse() {
            out.push(id);
        }
    }
    out
}

fn plain(out: &mut TcpStream, v: &Value) {
    let body = v.to_string();
    let _ = write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
}

/// The text as server-sent events, three characters a chunk, so citations
/// arrive cut in half.
fn streamed(out: &mut TcpStream, text: &str) {
    let _ = out.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
    );
    let chars: Vec<char> = text.chars().collect();
    let mut events: Vec<String> = chars
        .chunks(3)
        .map(|c| {
            let piece: String = c.iter().collect();
            format!(
                "data: {}\n\n",
                json!({"choices": [{"delta": {"content": piece}}]})
            )
        })
        .collect();
    events.push(format!(
        "data: {}\n\n",
        json!({"choices": [{"delta": {}, "finish_reason": "stop"}],
               "timings": {"prompt_n": 500, "predicted_n": 40,
                           "prompt_per_second": 2000.0, "predicted_per_second": 60.0}})
    ));
    events.push("data: [DONE]\n\n".into());
    for e in events {
        let _ = write!(out, "{:x}\r\n{e}\r\n", e.len());
        let _ = out.flush();
    }
    let _ = out.write_all(b"0\r\n\r\n");
}

/// `per_day` messages on each of `days` days from 2024-12-01, in two topics.
fn chat(days: u32, per_day: i64) -> Vec<Row> {
    let mut rows = Vec::new();
    for d in 0..days {
        for i in 0..per_day {
            let id = 1000 + d as i64 * 100 + i;
            rows.push(Row {
                id,
                t: format!("2024-12-{:02} {:02}:{:02}", d + 1, 8 + i / 60, i % 60),
                topic: (i % 2) as usize,
                who: format!("Person {}", i % 5),
                text: format!("PRIVATE-TEXT message {id} about the plan for the week"),
                ..Default::default()
            });
        }
    }
    rows
}

fn topics() -> Vec<String> {
    vec!["General".into(), "Media".into()]
}

fn request<'a>(
    rows: &'a [Row],
    topics: &'a [String],
    length: Length,
    by_topic: bool,
) -> Request<'a> {
    Request {
        chat: "Group",
        rows,
        topics,
        forum: true,
        by_topic,
        length,
        lang: Lang::English,
        from: NaiveDate::from_ymd_opt(2024, 12, 1).unwrap(),
        to: NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
    }
}

fn run(fake: &Fake, req: &Request, cache: &Cache) -> (Result<tga_sum::Summary, Error>, Vec<Event>) {
    let mut events = Vec::new();
    let never = AtomicBool::new(false);
    let out = summarise(&fake.client(), req, cache, &never, &mut |e| events.push(e));
    (out, events)
}

fn streamed_text(events: &[Event]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_range_that_fits_is_one_streamed_call_and_a_stranger_is_never_cited() {
    let fake = Fake::start(32_768);
    let rows = chat(2, 10);
    let topics = topics();
    let (out, events) = run(
        &fake,
        &request(&rows, &topics, Length::Detailed, false),
        &Cache::none(),
    );
    let s = out.unwrap();
    assert_eq!(s.parts, 0);
    assert_eq!(fake.notes_calls(), 0);
    assert_eq!(fake.chats().len(), 1);
    assert_eq!(s.sections.len(), 1);
    assert_eq!(s.sections[0].text, "It was decided [#1000] and denied.");
    assert_eq!(
        streamed_text(&events),
        s.sections[0].text,
        "the stream is the text"
    );
    assert_eq!(s.model, "gemma-test.gguf");
    // Mixed topics: each line names its topic.
    let user = fake.chats()[0]
        .pointer("/messages/1/content")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert!(user.contains("#1000 08:00 [General] Person 0: "), "{user}");
    assert!(user.contains("== 2024-12-02 Mon =="));
}

#[test]
fn a_range_that_does_not_fit_goes_through_notes_and_the_cache_saves_a_rerun() {
    let fake = Fake::start(6_000);
    let rows = chat(6, 40);
    let topics = topics();
    let dir = std::env::temp_dir().join(format!("tga-sum-fake-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cache = Cache::at(&dir);
    let req = request(&rows, &topics, Length::Sections, false);

    let (out, events) = run(&fake, &req, &cache);
    let s = out.unwrap();
    assert!(s.parts > 1, "{} parts", s.parts);
    assert_eq!(fake.notes_calls(), s.parts);
    assert_eq!(s.failed, 0);
    let parts_done = events
        .iter()
        .filter(|e| matches!(e, Event::Part { cached: false, .. }))
        .count();
    assert_eq!(parts_done, s.parts);
    // The final call read notes, and only real ids reached them.
    let last = fake.chats().last().unwrap().clone();
    let notes = last
        .pointer("/messages/1/content")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(notes.starts_with("Notes:"), "{notes}");
    assert!(!notes.contains(&STRANGER.to_string()));
    assert!(!s.sections[0].text.contains(&STRANGER.to_string()));

    // The same range again: every part from the cache, one call in all.
    let before = fake.chats().len();
    let (again, events) = run(&fake, &req, &cache);
    assert_eq!(again.unwrap().sections, s.sections);
    assert_eq!(fake.chats().len(), before + 1);
    assert!(events
        .iter()
        .all(|e| !matches!(e, Event::Part { cached: false, .. })));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_notes_answer_that_is_not_json_is_asked_again() {
    let fake = Fake::start(6_000);
    fake.script.garbage.store(2, Ordering::SeqCst);
    let rows = chat(6, 40);
    let topics = topics();
    let (out, _) = run(
        &fake,
        &request(&rows, &topics, Length::Short, false),
        &Cache::none(),
    );
    let s = out.unwrap();
    assert_eq!(s.failed, 0);
    assert_eq!(fake.notes_calls(), s.parts + 2);
}

#[test]
fn a_part_that_never_answers_is_counted_and_left_out() {
    let fake = Fake::start(6_000);
    // Three bad answers: the first part's every try.
    fake.script.garbage.store(3, Ordering::SeqCst);
    let rows = chat(6, 40);
    let topics = topics();
    let (out, _) = run(
        &fake,
        &request(&rows, &topics, Length::Short, false),
        &Cache::none(),
    );
    assert_eq!(out.unwrap().failed, 1);
}

#[test]
fn by_topic_writes_each_topic_and_an_overview_above_them() {
    let fake = Fake::start(32_768);
    let rows = chat(2, 10);
    let topics = topics();
    let (out, events) = run(
        &fake,
        &request(&rows, &topics, Length::Detailed, true),
        &Cache::none(),
    );
    let s = out.unwrap();
    let heads: Vec<_> = s.sections.iter().map(|x| x.heading.clone()).collect();
    assert_eq!(
        heads,
        vec![None, Some("General".into()), Some("Media".into())]
    );
    assert!(events.contains(&Event::Section(Some("Media".into()))));
    // A topic's lines carry no tag: its name is in the prompt instead.
    let user = fake.chats()[0]
        .pointer("/messages/1/content")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert!(!user.contains("[General]"));
    let system = fake.chats()[0]
        .pointer("/messages/0/content")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert!(system.contains("the topic \"General\""), "{system}");
    // The overview cites nothing.
    assert!(!s.sections[0].text.contains("[#"));
}

#[test]
fn the_narration_never_carries_a_message() {
    let fake = Fake::start(6_000);
    let rows = chat(6, 40);
    let topics = topics();
    let (out, events) = run(
        &fake,
        &request(&rows, &topics, Length::Detailed, true),
        &Cache::none(),
    );
    out.unwrap();
    let said: Vec<&String> = events
        .iter()
        .filter_map(|e| match e {
            Event::Say(s) => Some(s),
            _ => None,
        })
        .collect();
    assert!(!said.is_empty());
    for s in said {
        assert!(!s.contains("PRIVATE-TEXT") && !s.contains("Person"), "{s}");
    }
}

#[test]
fn stop_ends_a_run_mid_stream() {
    let fake = Fake::start(32_768);
    let rows = chat(1, 5);
    let topics = topics();
    let cancel = AtomicBool::new(false);
    let mut pieces = 0;
    let out = summarise(
        &fake.client(),
        &request(&rows, &topics, Length::Short, false),
        &Cache::none(),
        &cancel,
        &mut |e| {
            if matches!(e, Event::Text(_)) {
                pieces += 1;
                cancel.store(true, Ordering::Relaxed);
            }
        },
    );
    assert!(matches!(out, Err(Error::Cancelled)), "{out:?}");
    assert_eq!(pieces, 1);
}

#[test]
fn no_server_is_said_plainly() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let rows = chat(1, 5);
    let topics = topics();
    let never = AtomicBool::new(false);
    let out = summarise(
        &Client::new(port),
        &request(&rows, &topics, Length::Short, false),
        &Cache::none(),
        &never,
        &mut |_| {},
    );
    assert!(
        matches!(out, Err(Error::NoServer(p)) if p == port),
        "{out:?}"
    );
}

#[test]
fn photos_are_read_as_revisions_ocr_asks_and_cleaned() {
    let fake = Fake::start(32_768);
    let jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 1, 2, 3];
    let mut photos: Vec<Photo> = (0..9)
        .map(|i| Photo {
            key: format!("p{i}"),
            bytes: jpeg.clone(),
        })
        .collect();
    photos.push(Photo {
        key: "webp".into(),
        bytes: b"RIFF\0\0\0\0WEBPVP8 ".to_vec(),
    });
    let never = AtomicBool::new(false);
    let read = read_all(&fake.client(), photos, &never).unwrap();

    assert_eq!(read.len(), 10);
    assert_eq!(
        read["p0"],
        "ПЛАКАТ: ЉУДИ ИСПРЕД РЕКТОРАТА u petak u podne u podne"
    );
    // What the server cannot decode is not sent, and reads as no text.
    assert_eq!(read["webp"], "");
    let sent = fake.chats();
    assert_eq!(sent.len(), 9);
    for body in sent {
        assert_eq!(body["temperature"], 0);
        assert_eq!(
            body.pointer("/messages/0/content/1/text"),
            Some(&json!("OCR:"))
        );
        let url = body
            .pointer("/messages/0/content/0/image_url/url")
            .and_then(Value::as_str)
            .unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,/9j/"), "{url}");
    }
}

#[test]
fn a_question_is_answered_from_the_days_with_the_conversation_and_checked_citations() {
    let fake = Fake::start(32_768);
    let rows = chat(2, 6);
    let topics = topics();
    let never = AtomicBool::new(false);
    let req = request(&rows, &topics, Length::Short, true);
    let g = ground(&fake.client(), &req, &Cache::none(), &never, &mut |_| {}).unwrap();
    // Small enough to read whole, and never split by topic.
    assert!(!g.notes);
    assert_eq!(g.messages, rows.len());
    assert_eq!(fake.chats().len(), 0, "reading the days asked nothing yet");

    let history = vec![Turn {
        question: "Earlier?".into(),
        answer: "Earlier answer.".into(),
    }];
    let mut streamed = String::new();
    let a = answer(
        &fake.client(),
        &req,
        &g,
        &history,
        "Who decided?",
        &never,
        &mut |e| {
            if let Event::Text(t) = e {
                streamed.push_str(&t);
            }
        },
    )
    .unwrap();
    assert_eq!(a, streamed.trim());
    assert!(a.contains(&format!("[#{}]", rows[0].id)), "{a}");
    assert!(!a.contains(&STRANGER.to_string()), "{a}");

    let sent = fake.chats();
    let messages = sent[0]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4, "system, the earlier turn, the question");
    assert_eq!(messages[1]["content"], "Earlier?");
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(messages[3]["content"], "Who decided?");
    let system = messages[0]["content"].as_str().unwrap();
    assert!(system.contains(&format!("#{} ", rows[0].id)), "{system}");
}

/// Ground a month too long to read whole, so a question goes through notes.
fn ground_on_notes(fake: &Fake, rows: &[Row], topics: &[String]) -> tga_sum::Grounding {
    let never = AtomicBool::new(false);
    let req = request(rows, topics, Length::Short, false);
    let g = ground(&fake.client(), &req, &Cache::none(), &never, &mut |_| {}).unwrap();
    assert!(g.notes, "{} messages were read whole", rows.len());
    assert!(!g.listed.is_empty() && !g.days.is_empty());
    g
}

fn ask(
    fake: &Fake,
    rows: &[Row],
    topics: &[String],
    g: &tga_sum::Grounding,
) -> (String, Vec<Event>) {
    let never = AtomicBool::new(false);
    let req = request(rows, topics, Length::Short, false);
    let mut events = Vec::new();
    let a = answer(
        &fake.client(),
        &req,
        g,
        &[],
        "Who was for it?",
        &never,
        &mut |e| events.push(e),
    )
    .unwrap();
    (a, events)
}

#[test]
fn a_question_over_notes_reads_the_messages_behind_the_notes_it_picks() {
    let fake = Fake::start(12_000);
    let mut rows = chat(20, 40);
    // A message far from the first note's that it answers: read too.
    rows[2].re = Some(rows[500].id);
    let topics = topics();
    let g = ground_on_notes(&fake, &rows, &topics);
    let before = fake.chats().len();

    let (a, events) = ask(&fake, &rows, &topics, &g);
    let sent = fake.chats();
    assert_eq!(sent.len(), before + 2, "a pick, then the answer");
    let pick = &sent[before];
    let user = pick
        .pointer("/messages/1/content")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(user.starts_with("Notes:\n\n1. 2024-12-"), "{user}");
    assert!(user.ends_with("Newest question: Who was for it?"), "{user}");

    // The first note rests on #1000: it and three either side are read, the
    // message one of them answers, and nothing else.
    let system = sent[before + 1]
        .pointer("/messages/0/content")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(system.contains("Notes, for orientation:"), "{system}");
    for id in [1000, 1001, 1002, 1003, rows[500].id] {
        assert!(system.contains(&format!("\n#{id} ")), "#{id}: {system}");
    }
    assert!(!system.contains("\n#1004 "), "{system}");
    assert!(system.contains("\n…\n"), "{system}");
    assert!(a.contains("[#1000]"), "{a}");
    assert!(!a.contains(&STRANGER.to_string()), "{a}");
    assert!(events.iter().any(
        |e| matches!(e, Event::Say(s) if s.starts_with("Looking up the messages behind 1 notes"))
    ));
}

#[test]
fn a_question_whose_pick_finds_nothing_is_answered_from_every_note() {
    let fake = Fake::start(12_000);
    fake.script.pick_none.store(true, Ordering::SeqCst);
    let rows = chat(20, 40);
    let topics = topics();
    let g = ground_on_notes(&fake, &rows, &topics);
    let (a, _) = ask(&fake, &rows, &topics, &g);
    let system = fake.chats().last().unwrap()["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        system.contains(&format!("Notes:\n\n{}", g.text)),
        "{system}"
    );
    assert!(!system.contains("\n#1001 "), "{system}");
    assert!(a.contains("[#1000]"), "{a}");
}

#[test]
fn a_photos_text_reaches_the_model_in_its_line() {
    let fake = Fake::start(32_768);
    let mut rows = chat(1, 6);
    rows[2].media = "photo".into();
    rows[2].photo_text = "Protest ispred Rektorata u petak u podne".into();
    let topics = topics();
    let never = AtomicBool::new(false);
    summarise(
        &fake.client(),
        &request(&rows, &topics, Length::Short, false),
        &Cache::none(),
        &never,
        &mut |_| {},
    )
    .unwrap();
    let user = fake.chats()[0]
        .pointer("/messages/1/content")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    assert!(
        user.contains("[photo with text: Protest ispred Rektorata u petak u podne]"),
        "{user}"
    );
}

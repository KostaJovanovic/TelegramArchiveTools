//! `tgx unpack` against the real export, if the corpus is here.
//!
//! The archive leg proves a reference `result.json` survives a store; this
//! proves the *unpack* does — every reference topic merged into a database of
//! its own, written back out through `tgx_tg::unpack`, and byte-compared with
//! Desktop's file. It lives here and not in `tgx-parity` because the harness
//! may not depend on `tgx-tg`, which is where the unpack is.
//!
//! Gated exactly as `crates/tgx-parity/tests/corpus.rs` is: no `reference/`
//! skips, unless `TGX_REQUIRE_CORPUS` is set to anything but `0`.
//! `TGX_CORPUS_DIR` points it at a corpus elsewhere — a worktree has none of
//! its own, the directory being gitignored.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use tgx_archive::Store;
use tgx_tg::config::Settings;

fn corpus_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("TGX_CORPUS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference"));
    if dir.join("MANIFEST.txt").is_file() {
        return Some(dir);
    }
    match std::env::var("TGX_REQUIRE_CORPUS").ok().as_deref() {
        Some(v) if !v.is_empty() && v != "0" => panic!(
            "TGX_REQUIRE_CORPUS is set, so a missing corpus at {} is a failure",
            dir.display()
        ),
        _ => {
            eprintln!("no corpus at {} — the unpack is not covered", dir.display());
            None
        }
    }
}

#[tokio::test]
async fn every_reference_topic_unpacks_to_desktops_own_bytes() {
    let Some(dir) = corpus_dir() else { return };
    let mut topics: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("reading the corpus")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("result.json").is_file())
        .collect();
    topics.sort();
    assert!(
        !topics.is_empty(),
        "a corpus with no topics compares nothing"
    );

    let work = std::env::temp_dir().join(format!("tgx-unpack-corpus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    let settings = Settings {
        export_html: false,
        export_json: true,
        ..Settings::default()
    };

    for (n, topic) in topics.iter().enumerate() {
        let text = std::fs::read_to_string(topic.join("result.json"))
            .unwrap()
            .replace("\r\n", "\n");
        let parsed: Map<String, Value> = serde_json::from_str(&text).unwrap();
        let name = parsed["name"].as_str().unwrap_or("").to_string();
        let kind = parsed["type"].as_str().unwrap_or("").to_string();
        let id = parsed["id"].as_i64().unwrap();
        // Whatever follows Desktop's three keys is what `Output::new` was
        // handed as `extra_head` — the archive leg's `head_of`.
        let head: Map<String, Value> = parsed
            .iter()
            .filter(|(k, _)| !matches!(k.as_str(), "name" | "type" | "id" | "messages"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let db = work.join(format!("db{n}")).join("chat.sqlite");
        {
            let store = Store::open(&db).await.unwrap();
            store.upsert_chat(id, &name, &kind, 1).await.unwrap();
            store.upsert_topic(id, 1, &name, &head, 1).await.unwrap();
            store.begin_batch().await.unwrap();
            for m in parsed["messages"].as_array().unwrap() {
                let m = m.as_object().unwrap();
                let mid = m["id"].as_i64().unwrap();
                store.merge(id, mid, None, m, 1).await.unwrap();
            }
            store.commit_batch().await.unwrap();
        }

        let done = tgx_tg::unpack::write_classic(&db, &work.join(format!("out{n}")), &settings)
            .await
            .unwrap();
        let ours = std::fs::read_to_string(done[0].root.join("result.json")).unwrap();
        assert!(
            ours == text,
            "{}: the unpacked result.json differs from Desktop's",
            topic.display()
        );
    }
    let _ = std::fs::remove_dir_all(&work);
}

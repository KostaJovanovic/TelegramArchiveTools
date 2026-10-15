//! Reading a tree, pairing what is in it, and classifying one difference.
//!
//! A difference is classified before it is counted: a rename is not a
//! mismatch, a gap the export stated is not a dangling path, and a value that
//! moved between two runs is not a field we never wrote.

use super::*;

/// Two non-empty names for the same person: somebody renamed their profile.
///
/// An empty name on either side is **not** a rename — that is the 206-people
/// bug this field is in [`MUST_MATCH`] to catch, and it must keep failing.
pub(super) fn is_rename(x: Option<&Value>, y: Option<&Value>) -> bool {
    let (Some(a), Some(b)) = (x.and_then(Value::as_str), y.and_then(Value::as_str)) else {
        return false;
    };
    !a.trim().is_empty() && !b.trim().is_empty()
}

/// Which person a display-name disagreement belongs to.
///
/// A rename is uniform across one peer's messages; anything that is not is a
/// different bug wearing the same clothes, and bucketing on the id is what tells
/// them apart. `forwarded_from` carries no id of its own, so it buckets on the
/// name the reference gave.
pub(super) fn peer_key(reference: &Map<String, Value>, field: &str) -> String {
    let id = match field {
        "from" => reference.get("from_id"),
        "actor" => reference.get("actor_id"),
        _ => None,
    };
    if let Some(id) = id.and_then(Value::as_str) {
        return id.to_string();
    }
    match reference.get(field).and_then(Value::as_str) {
        Some(name) => format!("{field}={name}"),
        None => format!("{field}=?"),
    }
}

/// Paths the export itself declared it could not save.
///
/// A path occupies a whole line and starts in column zero; an **indented** line
/// is the reason for the path above it (`tgx_tg::download::write_missing`).
/// Filtering on the indent rather than on a separator inside the line is what
/// keeps the split unambiguous when a Telegram filename contains the separator.
pub(super) fn stated_gaps(topic: &Path) -> BTreeSet<String> {
    let Ok(body) = std::fs::read_to_string(topic.join("missing_media.txt")) else {
        return BTreeSet::new();
    };
    body.lines()
        .filter(|l| !l.starts_with(char::is_whitespace))
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("These files are referenced"))
        .map(str::to_string)
        .collect()
}

/// What one export said happened to one media field.
///
/// Three states, and the leg used to have two. `skip_reason` returned `None`
/// for "the key is absent" *and* for "the key holds a path", and the example
/// line then rendered that `None` as the word `"downloaded"` — so a message
/// with no `photo` key at all was reported as a photo we had downloaded. That
/// is the wire leg inventing the other side's answer, in the one report whose
/// entire job is to say what the other side actually holds.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decision<'a> {
    /// The key is not in the message.
    Absent,
    /// A path: the bytes were saved.
    Saved,
    /// One of Desktop's parenthesised notes, verbatim.
    Skipped(&'a str),
}

impl Decision<'_> {
    pub(super) fn is_skip(&self) -> bool {
        matches!(self, Decision::Skipped(_))
    }

    pub(super) fn describe(&self) -> String {
        match self {
            Decision::Absent => "absent".into(),
            Decision::Saved => "downloaded".into(),
            Decision::Skipped(why) => format!("{why:?}"),
        }
    }

    /// The class of decision, for grouping. Substring tests rather than equality
    /// against Desktop's exact sentences, matching `tgx_html::media`.
    pub(super) fn class(&self) -> &'static str {
        match self {
            Decision::Absent => "absent",
            Decision::Saved => "saved",
            Decision::Skipped(why) => {
                let lowered = why.to_ascii_lowercase();
                if lowered.contains("exceeds maximum size") {
                    "too_large"
                } else if lowered.contains("not included") {
                    "not_included"
                } else {
                    "unavailable"
                }
            }
        }
    }
}

pub(super) fn decision<'a>(m: &'a Map<String, Value>, key: &str) -> Decision<'a> {
    match m.get(key).and_then(Value::as_str) {
        None => Decision::Absent,
        Some(v) if tgx_format::placeholder::is_skipped(v) => Decision::Skipped(v),
        Some(_) => Decision::Saved,
    }
}

/// Fields whose value is somebody's words or name rather than the format's.
///
/// An example line shows these as their shape only, unless
/// [`crate::SHOW_TEXT`] is set; the rest of [`MUST_MATCH`] is dates, ids,
/// sizes and Desktop's own vocabulary, and is shown as it is.
const PERSONAL: &[&str] = &["text", "from", "actor", "forwarded_from", "file_name"];

/// One side of an example line.
pub(super) fn brief(field: &str, v: Option<&Value>, show: bool) -> String {
    let Some(v) = v else { return "absent".into() };
    if !show && PERSONAL.contains(&field) {
        return shape(v);
    }
    let s = v.to_string();
    if s.chars().count() > 60 {
        let cut: String = s.chars().take(57).collect();
        format!("{cut}…")
    } else {
        s
    }
}

/// What a value is, without what it says: enough to tell an empty name from a
/// different one, or a plain string from a `text` array.
pub(super) fn shape(v: &Value) -> String {
    match v {
        Value::String(s) if s.is_empty() => "\"\"".into(),
        Value::String(s) => format!("a string of {} chars", s.chars().count()),
        Value::Array(a) => format!("an array of {}", a.len()),
        Value::Object(o) => format!("an object of {} keys", o.len()),
        // A number, a bool or null spells nothing.
        other => other.to_string(),
    }
}

/// A media path with the name Telegram's sender gave it left out: its folder
/// and its extension are the exporter's, the rest is theirs.
pub(super) fn path_shape(path: &str, show: bool) -> String {
    if show {
        return path.to_string();
    }
    let (folder, file) = path.rsplit_once('/').unwrap_or(("", path));
    let ext = file
        .rsplit_once('.')
        .map(|(_, e)| format!(".{e}"))
        .filter(|e| e.chars().count() <= 8)
        .unwrap_or_default();
    format!("{folder}/<{} chars>{ext}", file.chars().count())
}

pub(super) fn messages_by_id(topic: &Path) -> Result<BTreeMap<i64, Map<String, Value>>> {
    let path = topic.join("result.json");
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let data: Value = serde_json::from_str(&raw)?;
    let messages = data
        .get("messages")
        .and_then(Value::as_array)
        .with_context(|| format!("{} has no messages array", path.display()))?;
    let mut out = BTreeMap::new();
    for m in messages {
        let Some(map) = m.as_object() else { continue };
        let Some(id) = map.get("id").and_then(Value::as_i64) else {
            continue;
        };
        out.insert(id, map.clone());
    }
    Ok(out)
}

/// Topic folders keyed by the `name` **inside** `result.json`, not by folder.
///
/// A forum export is one folder per topic, but the two sides do not name those
/// folders alike: Desktop uses the bare topic title, we prefix the topic id, so
/// `ćaskanje` and `0001 - ćaskanje` are one topic under two names. Keying on the
/// folder made every real comparison degenerate — against the reference run all
/// eight folders came back "only in ours" and "we did not export this topic",
/// so the leg reported eight failures having compared no message at all. That
/// reads like a total export failure rather than a harness that cannot pair its
/// inputs. The title inside the file is the thing both sides agree on.
///
/// **Two folders with one title are an error, not an overwrite.** Forum topics
/// may share a title, and our folders keep them apart by the id prefix
/// (`0001 - x`, `0002 - x`) while each records the bare title. Inserted
/// blindly, the later folder replaced the earlier and that topic was never
/// read — a leg reporting zero missing ids for messages it never compared,
/// which is the harness failing open again. Nothing on Desktop's side names the
/// topic id, so there is no honest way to pair such a pair; saying so is the
/// most the leg can do.
pub(super) fn topics_by_name(root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let folders = crate::topic_folders(root)?;
    let mut out: BTreeMap<String, PathBuf> = BTreeMap::new();
    for f in &folders {
        let folder = f
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let name = std::fs::read_to_string(f.join("result.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.get("name")?.as_str().map(str::to_string))
            .unwrap_or(folder);
        if let Some(earlier) = out.get(&name) {
            anyhow::bail!(
                "{} and {} both record the topic title {name:?}, so the wire leg cannot \
                 tell which to compare with which; compare them one folder at a time",
                earlier.display(),
                f.display()
            );
        }
        out.insert(name, f.clone());
    }
    Ok(out)
}

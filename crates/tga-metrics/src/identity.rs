//! Who is who, across a history in which people rename themselves.
//!
//! One person is one **typed peer key** (`user123`), never a display name.
//! Telegram lets anyone change their name at any moment and an export records
//! whatever they were called when each message was sent, so grouping by the
//! string splits one person into as many rows as they had names — and, in a
//! group where two people picked the same name, merges two people into one.
//!
//! The key is stable, so it is the identity. The name is presentation, and the
//! report shows the **newest** one: that is who you would recognise today.
//! Every earlier name is kept in `aliases`, newest first, because the older
//! name is often the one you actually remember them by.
//!
//! Newest is decided by the last message the person sent, not by the roster:
//! `participants.json` holds one current name per member and cannot date it,
//! so it is a fallback for people who never spoke, not evidence about a
//! rename.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use tga_read::{Export, Msg};

#[derive(Debug, Clone, Default)]
pub struct Person {
    pub key: String,
    /// What they are called now.
    pub name: String,
    /// Every earlier name, newest first. Empty for someone who never renamed.
    pub aliases: Vec<String>,
    pub role: String,
    pub joined: Option<NaiveDateTime>,
    /// True when the roster names them but no message in the export does.
    pub silent: bool,
}

impl Person {
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            &self.key
        } else {
            &self.name
        }
    }
}

/// Peer key -> [`Person`], built once and shared by every metric.
///
/// Insertion order is preserved, because it is observable: `people.rows` is
/// sorted by `(-messages, name.lower())` with a *stable* sort, so first-seen
/// order breaks the remaining ties.
pub struct People {
    order: Vec<Person>,
    index: HashMap<String, usize>,
}

impl People {
    pub fn new(export: &Export) -> Self {
        let mut this = People {
            order: Vec::new(),
            index: HashMap::new(),
        };
        this.build(export);
        this
    }

    fn build(&mut self, export: &Export) {
        // Names in the order they were used. The export is already sorted
        // oldest-first, so the last name seen for a key is the newest.
        let mut seen_order: Vec<String> = Vec::new();
        let mut seen: HashMap<String, Vec<String>> = HashMap::new();

        for msg in &export.msgs {
            if msg.sender.is_empty() || msg.name.is_empty() {
                continue;
            }
            let names = seen.entry(msg.sender.clone()).or_insert_with(|| {
                seen_order.push(msg.sender.clone());
                Vec::new()
            });
            if names.last().map(String::as_str) != Some(msg.name.as_str()) {
                names.push(msg.name.clone());
            }
        }

        // A reactor may never have spoken, and a forward names a peer that is
        // not a member at all. Both are people the report has to render.
        for msg in &export.msgs {
            for reaction in &msg.reactions {
                for key in &reaction.named {
                    seen.entry(key.clone()).or_insert_with(|| {
                        seen_order.push(key.clone());
                        Vec::new()
                    });
                }
            }
        }

        for key in seen_order {
            let names = &seen[&key];
            // Newest first, duplicates dropped.
            let mut ordered: Vec<String> = Vec::new();
            for name in names.iter().rev() {
                if !ordered.contains(name) {
                    ordered.push(name.clone());
                }
            }
            let name = ordered.first().cloned().unwrap_or_else(|| key.clone());
            let aliases = ordered.into_iter().skip(1).collect();
            self.push(Person {
                key,
                name,
                aliases,
                ..Default::default()
            });
        }

        for member in &export.roster {
            match self.index.get(&member.key).copied() {
                None => self.push(Person {
                    key: member.key.clone(),
                    name: member.name.clone(),
                    silent: true,
                    ..Default::default()
                }),
                Some(at) => {
                    if self.order[at].name.is_empty() {
                        self.order[at].name = member.name.clone();
                    }
                }
            }
            let at = self.index[&member.key];
            self.order[at].role = member.role.clone();
            self.order[at].joined = member.joined;
            // The roster's name is current by definition. If the person's last
            // message used a different one, that message is the older fact.
            if !member.name.is_empty() && self.order[at].name != member.name {
                let previous = std::mem::replace(&mut self.order[at].name, member.name.clone());
                let mut aliases = vec![previous];
                aliases.extend(
                    self.order[at]
                        .aliases
                        .iter()
                        .filter(|a| *a != &member.name)
                        .cloned(),
                );
                self.order[at].aliases = aliases;
            }
        }
    }

    fn push(&mut self, person: Person) {
        self.index.insert(person.key.clone(), self.order.len());
        self.order.push(person);
    }

    /// The person behind a key.
    ///
    /// The Python version *inserts* unknown keys on lookup. Nothing observable
    /// depends on that: the only iteration over `People` filters on `silent`,
    /// which a synthesised entry never is, and every caller of `name_of`
    /// already has its own row for the key. So this stays non-mutating and
    /// hands back a stand-in instead.
    pub fn get(&self, key: &str) -> Person {
        match self.index.get(key) {
            Some(&at) => self.order[at].clone(),
            None => Person {
                key: key.to_string(),
                name: if key.is_empty() {
                    "unknown".to_string()
                } else {
                    key.to_string()
                },
                ..Default::default()
            },
        }
    }

    pub fn name_of(&self, key: &str) -> String {
        match self.index.get(key) {
            Some(&at) => self.order[at].label().to_string(),
            None if key.is_empty() => "unknown".to_string(),
            None => key.to_string(),
        }
    }

    /// The identity a message belongs to.
    ///
    /// Falls back to the display name for a message with no `from_id` at all —
    /// a channel post signed only by its author, say. Prefixed so it can never
    /// collide with a real peer key.
    pub fn key_of(&self, msg: &Msg) -> String {
        if !msg.sender.is_empty() {
            msg.sender.clone()
        } else if !msg.name.is_empty() {
            format!("name:{}", msg.name)
        } else {
            String::new()
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Person> {
        self.order.iter()
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn renamed(&self) -> impl Iterator<Item = &Person> {
        self.order.iter().filter(|p| !p.aliases.is_empty())
    }
}

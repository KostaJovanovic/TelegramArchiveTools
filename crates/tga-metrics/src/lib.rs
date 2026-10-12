//! Every figure the report can show, computed in one pass over the export.
//!
//! [`analyse`] fills in a [`tga_stats::Stats`] and hands it back. The shape
//! lives in that crate rather than here so the report can read one without
//! depending on this — see its module docs, and `tga-report/Cargo.toml`.

pub mod activity;
pub mod content;
pub mod conversation;
pub mod dynamics;
pub mod extras;
pub mod graph;
pub mod identity;
pub mod people;
pub mod util;

use tga_read::Export;
use tga_stats::{ExportInfo, Stats};

pub use identity::{People, Person};

/// How many messages somebody needs before they can win a per-person award.
///
/// Without a floor every one of them is won by somebody who sent four messages,
/// all of them at 4am.
const AWARD_MINIMUM: i64 = 40;

pub fn analyse(export: &Export) -> (Stats, People) {
    let who = People::new(export);

    let activity = activity::compute(export, &who);
    let folk = people::compute(export, &who);
    let content = content::compute(export, &who);
    let talk = conversation::compute(export, &who);

    // The graph is laid out on how much each person said, so it needs the
    // people rows before it can be built. Nothing else here depends on
    // anything else here.
    let weights: std::collections::HashMap<String, i64> = folk
        .rows
        .iter()
        .map(|row| (row.key.clone(), row.messages))
        .collect();
    let net = graph::build(&talk.edges, &talk.reaction_edges, &weights);

    let mut renamed: Vec<&Person> = who.renamed().collect();
    renamed.sort_by_key(|p| p.name.to_lowercase());

    let stats = Stats {
        export: ExportInfo {
            name: export.name.clone(),
            root: util::path_string(&export.root),
            topics: export.topics.len(),
            messages: export.msgs.len(),
            service_messages: export.msgs.iter().filter(|m| m.service).count(),
            members_count: export.members_count,
            roster_complete: export.roster_complete,
        },
        topics: extras::topics(export, &who),
        superlatives: extras::superlatives(export, &who, &activity),
        awards: extras::awards(&folk.rows, AWARD_MINIMUM),
        streak: extras::streaks(export, &who),
        churn: extras::churn(export, &who, &activity),
        dynamics: Some(dynamics::compute(export, &who)),
        renamed: renamed
            .iter()
            .map(|p| tga_stats::Renamed {
                name: p.name.clone(),
                aliases: p.aliases.clone(),
            })
            .collect(),
        activity,
        people: folk,
        content,
        conversation: talk,
        graph: net,
    };

    (stats, who)
}

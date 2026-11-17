//! H5-only sanitized ranking projection. This module is absent from normal builds.

use std::collections::HashSet;

use serde::Serialize;

use crate::domain::Message;

pub const MAX_CANDIDATE_ALIASES: usize = 10_000;

#[derive(Debug, Serialize)]
pub struct RankedHit {
    pub alias: String,
    pub score: f64,
    pub rank: usize,
}

/// Validate corpus aliases before any search work. Alias values are the only
/// identifiers that can cross the diagnostic response boundary.
pub fn validate_aliases(aliases: &std::collections::HashMap<uuid::Uuid, String>) -> bool {
    if aliases.is_empty() || aliases.len() > MAX_CANDIDATE_ALIASES {
        return false;
    }

    let mut unique = HashSet::with_capacity(aliases.len());
    aliases.values().all(|alias| {
        let valid = alias.strip_prefix("candidate-").is_some_and(|suffix| {
            suffix.len() == 2 && suffix.bytes().all(|byte| byte.is_ascii_digit())
        });
        valid && unique.insert(alias)
    })
}

pub fn project(
    hits: Vec<(Message, f64)>,
    aliases: &std::collections::HashMap<uuid::Uuid, String>,
) -> Result<Vec<RankedHit>, ()> {
    if !validate_aliases(aliases) {
        return Err(());
    }

    hits.into_iter()
        .enumerate()
        .map(|(index, (message, score))| {
            aliases
                .get(&message.id)
                .map(|alias| RankedHit {
                    alias: alias.clone(),
                    score,
                    rank: index + 1,
                })
                .ok_or(())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::Value;

    fn message(id: uuid::Uuid) -> Message {
        Message {
            id,
            role: "assistant".into(),
            content: "sensitive content-marker".into(),
            timestamp: Utc::now(),
            metadata: serde_json::Map::new(),
            embedding: Some(vec![0.25; 384]),
        }
    }

    #[test]
    fn projection_contains_only_alias_score_and_rank() {
        let internal_id = uuid::Uuid::new_v4();
        let aliases = std::collections::HashMap::from([(internal_id, "candidate-01".to_owned())]);
        let encoded =
            serde_json::to_value(project(vec![(message(internal_id), 0.875)], &aliases).unwrap())
                .unwrap();
        let Value::Array(rows) = encoded else {
            panic!("expected result list");
        };
        assert_eq!(rows[0]["score"], 0.875);
        assert_eq!(rows[0]["rank"], 1);
        assert_eq!(rows[0]["alias"], "candidate-01");
        let keys: Vec<_> = rows[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["alias", "rank", "score"]);
        let text = rows[0].to_string();
        assert!(!text.contains(&internal_id.to_string()));
        assert!(!text.contains("content-marker"));
        assert!(!text.contains("embedding"));
    }

    #[test]
    fn rejects_missing_invalid_duplicate_and_unmapped_aliases() {
        let id = uuid::Uuid::new_v4();
        assert!(project(vec![(message(id), 0.5)], &std::collections::HashMap::new()).is_err());

        for alias in ["bad", "candidate-1", "candidate-001"] {
            let aliases = std::collections::HashMap::from([(id, alias.to_owned())]);
            assert!(project(vec![(message(id), 0.5)], &aliases).is_err());
        }

        let other = uuid::Uuid::new_v4();
        let aliases = std::collections::HashMap::from([
            (id, "candidate-01".to_owned()),
            (other, "candidate-01".to_owned()),
        ]);
        assert!(!validate_aliases(&aliases));
        assert!(project(vec![(message(id), 0.5)], &aliases).is_err());

        let aliases = std::collections::HashMap::from([(other, "candidate-01".to_owned())]);
        assert!(project(vec![(message(id), 0.5)], &aliases).is_err());
    }
}

//! Long-term memory Cypher, ported from upstream `graph/queries.py` and
//! `graph/query_builder.py` (neo4j-labs/agent-memory @ 9d45af55, Apache-2.0).
//!
//! Statements marked "Muninn" have no bolt counterpart upstream (the
//! hosted-only Platinum entity operations) and follow the same graph shape.

use std::sync::LazyLock;

/// `entity_keys_set_clause`: `SET` items refreshing the resolver lookup keys.
///
/// `name_key` is the lower-cased name; `surface_keys` is every surface form
/// (name, canonical name, aliases) lower-cased and wrapped in `|` delimiters.
pub fn entity_keys_set_clause(var: &str) -> String {
    format!(
        "{var}.name_key = toLower(toString({var}.name)),\n    \
         {var}.surface_keys = reduce(acc = '|', form IN \
         [{var}.name, {var}.canonical_name] + coalesce({var}.aliases, []) | \
         CASE WHEN form IS NULL THEN acc \
         ELSE acc + replace(toLower(toString(form)), '|', ' ') + '|' END)"
    )
}

/// Known subtypes per POLE+O type (`query_builder.VALID_SUBTYPES`).
const VALID_SUBTYPES: &[(&str, &[&str])] = &[
    (
        "PERSON",
        &[
            "INDIVIDUAL",
            "ALIAS",
            "PERSONA",
            "SUSPECT",
            "WITNESS",
            "VICTIM",
        ],
    ),
    (
        "OBJECT",
        &[
            "VEHICLE", "PHONE", "EMAIL", "DOCUMENT", "DEVICE", "WEAPON", "MONEY", "DRUG",
            "EVIDENCE", "SOFTWARE", "PRODUCT",
        ],
    ),
    (
        "LOCATION",
        &[
            "ADDRESS",
            "CITY",
            "REGION",
            "COUNTRY",
            "LANDMARK",
            "FACILITY",
            "COORDINATES",
            "GEOPOLITICAL",
            "GEOGRAPHIC",
        ],
    ),
    (
        "EVENT",
        &[
            "INCIDENT",
            "MEETING",
            "TRANSACTION",
            "COMMUNICATION",
            "CRIME",
            "TRAVEL",
            "EMPLOYMENT",
            "OBSERVATION",
            "DATE",
            "TIME",
        ],
    ),
    (
        "ORGANIZATION",
        &[
            "COMPANY",
            "NONPROFIT",
            "GOVERNMENT",
            "EDUCATIONAL",
            "CRIMINAL",
            "POLITICAL",
            "RELIGIOUS",
            "MILITARY",
            "GROUP",
        ],
    ),
];

/// `to_pascal_case`: `PERSON` -> `Person`, `my_type` -> `MyType`.
fn to_pascal_case(value: &str) -> String {
    value
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect()
            })
        })
        .collect()
}

/// `sanitize_label`: a PascalCase label, or `None` when the value is not a
/// safe identifier (`^[A-Za-z][A-Za-z0-9_]*$`). This is what keeps dynamic
/// labels free of Cypher injection.
pub fn sanitize_label(value: &str) -> Option<String> {
    let stripped = value.trim();
    let mut chars = stripped.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    valid.then(|| to_pascal_case(stripped))
}

/// `validate_subtype`: POLE+O types accept only their known subtypes;
/// custom types accept any safe identifier.
pub fn subtype_label(entity_type: &str, subtype: &str) -> Option<String> {
    let kind = entity_type.to_uppercase();
    match VALID_SUBTYPES.iter().find(|(name, _)| *name == kind) {
        Some((_, known)) => known
            .contains(&subtype.to_uppercase().as_str())
            .then(|| to_pascal_case(subtype)),
        None => sanitize_label(subtype),
    }
}

/// `build_label_set_clause` without an ontology label.
pub fn label_set_clause(entity_type: &str, subtype: Option<&str>, var: &str) -> Option<String> {
    let kind = sanitize_label(entity_type)?;
    let mut labels = vec![kind];
    if let Some(label) = subtype.and_then(|subtype| subtype_label(entity_type, subtype)) {
        labels.push(label);
    }
    Some(format!(
        "SET {}",
        labels
            .iter()
            .map(|label| format!("{var}:{label}"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// `build_create_entity_query(type, subtype, include_aliases=True)`: MERGE on
/// `(name, type)` and add the type/subtype labels.
pub fn create_entity_query(entity_type: &str, subtype: Option<&str>) -> String {
    let (location_on_create, location_on_match) = if entity_type.eq_ignore_ascii_case("LOCATION") {
        (
            ",\n    e.location = CASE WHEN $location IS NOT NULL THEN point({latitude: $location.latitude, longitude: $location.longitude}) ELSE null END",
            ",\n    e.location = CASE WHEN $location IS NOT NULL THEN point({latitude: $location.latitude, longitude: $location.longitude}) ELSE e.location END",
        )
    } else {
        ("", "")
    };
    let mut query = format!(
        "MERGE (e:Entity {{name: $name, type: $type}})
ON CREATE SET
    e.id = $id,
    e.subtype = $subtype,
    e.canonical_name = $canonical_name,
    e.description = $description,
    e.embedding = $embedding,
    e.confidence = $confidence,
    e.created_at = datetime(),
    e.metadata = $metadata,
    e.aliases = coalesce($aliases, []){location_on_create}
ON MATCH SET
    e.subtype = COALESCE($subtype, e.subtype),
    e.canonical_name = COALESCE($canonical_name, e.canonical_name),
    e.description = COALESCE($description, e.description),
    e.embedding = COALESCE($embedding, e.embedding),
    e.updated_at = datetime(),
    e.aliases = coalesce(e.aliases, []) + [a IN coalesce($aliases, []) WHERE NOT a IN coalesce(e.aliases, [])]{location_on_match}
SET {}",
        entity_keys_set_clause("e")
    );
    if let Some(labels) = label_set_clause(entity_type, subtype, "e") {
        query.push('\n');
        query.push_str(&labels);
    }
    query.push_str("\nRETURN e");
    query
}

/// Single-entity textual blocking. Active, optionally same-type, bounded before
/// returning to the application. Lookup surfaces use the existing entity shape.
pub const RESOLUTION_LEXICAL_CANDIDATES: &str = "
MATCH (e:Entity)
WHERE e.merged_into IS NULL AND ($type IS NULL OR e.type = $type)
WITH e, [form IN [e.name, e.canonical_name] + coalesce(e.aliases, [])
    WHERE form IS NOT NULL | toLower(trim(form))] AS forms
WITH e, forms, CASE WHEN toLower(trim($name)) IN forms THEN 1.0 ELSE 0.0 END AS score
WHERE score = 1.0 OR any(form IN forms WHERE any(token IN split(toLower(trim($name)), ' ')
    WHERE token <> '' AND token IN split(form, ' ')))
RETURN e, score
ORDER BY score DESC, e.id ASC
LIMIT $limit
";

/// The vector bucket follows the pinned FIND_SIMILAR_ENTITIES_BY_EMBEDDING
/// shape, without swallowing index/dimension failures as an empty match set.
pub const RESOLUTION_VECTOR_CANDIDATES: &str = "
CALL db.index.vector.queryNodes('entity_embedding_idx', $limit, $embedding)
YIELD node, score
WHERE node.merged_into IS NULL AND ($type IS NULL OR node.type = $type)
RETURN node AS e, score
ORDER BY score DESC, node.id ASC
LIMIT $limit
";

pub const GET_ENTITY: &str = "
MATCH (e:Entity {id: $id})
RETURN e
";

pub const GET_ENTITY_BY_NAME: &str = "
MATCH (e:Entity)
WHERE e.merged_into IS NULL AND (e.name = $name OR e.canonical_name = $name OR $name IN COALESCE(e.aliases, []))
RETURN e
LIMIT 1
";

/// Filtering after vector candidate selection can return fewer than `limit`;
/// this query does not guarantee recall of all active entities.
pub const SEARCH_ENTITIES_BY_EMBEDDING: &str = "
CALL db.index.vector.queryNodes('entity_embedding_idx', $limit, $embedding)
YIELD node, score
WHERE score >= $threshold AND node.merged_into IS NULL
RETURN node AS e, score
ORDER BY score DESC
";

pub const SEARCH_ENTITIES_BY_TYPE: &str = "
MATCH (e:Entity {type: $type})
WHERE e.merged_into IS NULL
RETURN e
ORDER BY e.created_at DESC
LIMIT $limit
";

/// Muninn: every entity, newest first.
pub const LIST_ENTITIES: &str = "
MATCH (e:Entity)
WHERE e.merged_into IS NULL
RETURN e
ORDER BY e.created_at DESC
LIMIT $limit
";

/// Muninn: partial entity update that keeps the lookup keys in sync.
pub static UPDATE_ENTITY: LazyLock<String> = LazyLock::new(|| {
    format!(
        "
MATCH (e:Entity {{id: $id}})
SET e.name = coalesce($name, e.name),
    e.canonical_name = coalesce($canonical_name, e.canonical_name),
    e.description = coalesce($description, e.description),
    e.embedding = coalesce($embedding, e.embedding),
    e.updated_at = datetime()
SET {}
RETURN e
",
        entity_keys_set_clause("e")
    )
});

/// Muninn: remove an entity with every edge it carries.
pub const DELETE_ENTITY: &str = "
MATCH (e:Entity {id: $id})
DETACH DELETE e
RETURN true AS deleted
";

/// Muninn: human feedback on an entity (hosted `PUT /entities/{id}/feedback`).
pub const SET_ENTITY_FEEDBACK: &str = "
MATCH (e:Entity {id: $id})
SET e.user_score = $user_score,
    e.confirmed = $confirmed,
    e.feedback_at = datetime(),
    e.updated_at = datetime()
RETURN e.id AS id
";

/// Upstream `CREATE_ENTITY_RELATIONSHIP`.
///
/// Muninn additionally persists the
/// caller's free-form `properties` as a JSON string (upstream accepts
/// `attributes` but never stores them) and returns them with `created_at`.
pub const CREATE_ENTITY_RELATIONSHIP: &str = "
MATCH (e1:Entity {id: $source_id})
MATCH (e2:Entity {id: $target_id})
MERGE (e1)-[r:RELATED_TO {type: $relation_type}]->(e2)
ON CREATE SET
    r.id = $id,
    r.relation_type = $relation_type,
    r.description = $description,
    r.confidence = $confidence,
    r.support = 1,
    r.derived = $derived,
    r.extractor = $extractor,
    r.source_message_ids = CASE WHEN $message_id IS NULL THEN [] ELSE [$message_id] END,
    r.evidence = CASE WHEN $evidence IS NULL THEN [] ELSE [$evidence] END,
    r.valid_from = $valid_from,
    r.valid_until = $valid_until,
    r.properties = $properties,
    r.created_at = datetime()
ON MATCH SET
    r.confidence = CASE WHEN $confidence > r.confidence THEN $confidence ELSE r.confidence END,
    r.support = coalesce(r.support, 1) + 1,
    r.derived = coalesce(r.derived, false) AND $derived,
    r.source_message_ids = (coalesce(r.source_message_ids, []) +
        [x IN [$message_id] WHERE x IS NOT NULL AND NOT x IN coalesce(r.source_message_ids, [])])[..25],
    r.evidence = (coalesce(r.evidence, []) +
        [x IN [$evidence] WHERE x IS NOT NULL AND NOT x IN coalesce(r.evidence, [])])[..3],
    r.properties = coalesce($properties, r.properties),
    r.updated_at = datetime()
RETURN r.id AS id,
       r.description AS description,
       r.confidence AS confidence,
       r.properties AS properties,
       r.created_at AS created_at
";

/// Upstream `GET_ENTITY_RELATIONSHIPS` (undirected, one hop), plus the
/// Muninn `properties` column.
pub const GET_ENTITY_RELATIONSHIPS: &str = "
MATCH (e:Entity {id: $entity_id})-[r:RELATED_TO]-(other:Entity)
RETURN other,
       r.id AS rel_id,
       r.type AS rel_type,
       r.confidence AS confidence,
       r.support AS support,
       r.derived AS derived,
       r.description AS description,
       r.valid_from AS valid_from,
       r.valid_until AS valid_until,
       r.created_at AS created_at,
       r.updated_at AS updated_at,
       r.properties AS properties,
       type(r) AS neo4j_type
";

/// Muninn: entities within `depth` hops (`2..=MAX_RELATED_DEPTH`), each
/// reported once with the last edge of its shortest path.
///
/// Variable-length
/// bounds cannot be parameters, so the caller formats a validated integer.
pub fn entity_relationships_within(depth: u8) -> String {
    format!(
        "
MATCH path = (e:Entity {{id: $entity_id}})-[:RELATED_TO*1..{depth}]-(other:Entity)
WHERE other <> e
WITH other, path ORDER BY length(path) ASC
WITH other, head(collect(path)) AS path
WITH other, last(relationships(path)) AS r
RETURN other,
       r.id AS rel_id,
       r.type AS rel_type,
       r.confidence AS confidence,
       r.description AS description,
       r.created_at AS created_at,
       r.properties AS properties,
       type(r) AS neo4j_type
"
    )
}

/// Upstream `MERGE_ENTITIES`: copy the source's edges onto the target, mark
/// the source `merged_into` and record its name as a target alias.
pub static MERGE_ENTITIES: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{MERGE_ENTITIES_BODY}SET {}\nRETURN source, target\n",
        entity_keys_set_clause("target")
    )
});

const MERGE_ENTITIES_BODY: &str = "
MATCH (source:Entity {id: $source_id})
MATCH (target:Entity {id: $target_id})
WITH source, target, coalesce(source.merged_into = target.id, false) AS already_merged
CALL (source, target) {
    MATCH (source)<-[original:MENTIONS]-(m:Message)
    MERGE (m)-[mention:MENTIONS]->(target)
    ON CREATE SET mention = properties(original)
    WITH mention, original
    FOREACH (key IN [key IN keys(original) WHERE mention[key] IS NULL] |
        SET mention[key] = original[key])
    RETURN count(*) AS mentionsTransferred
}
CALL (source, target) {
    MATCH (source)-[r:SAME_AS]-(other:Entity)
    WHERE other <> target AND NOT (target)-[:SAME_AS]-(other)
    MERGE (target)-[:SAME_AS {
        confidence: r.confidence,
        match_type: 'merged',
        created_at: datetime()
    }]-(other)
    RETURN count(*) AS sameAsTransferred
}
CALL (source, target, already_merged) {
    MATCH (source)-[r:RELATED_TO]->(other:Entity)
    WHERE other <> target AND NOT already_merged
    WITH source, target, other, r,
         coalesce(r.type, r.relation_type, 'RELATED_TO') AS rel_type
    MERGE (target)-[nr:RELATED_TO {type: rel_type}]->(other)
    ON CREATE SET
        nr.id = r.id,
        nr.relation_type = rel_type,
        nr.description = r.description,
        nr.confidence = r.confidence,
        nr.support = coalesce(r.support, 1),
        nr.derived = coalesce(r.derived, false),
        nr.extractor = r.extractor,
        nr.source_message_ids = coalesce(r.source_message_ids, []),
        nr.evidence = coalesce(r.evidence, []),
        nr.valid_from = r.valid_from,
        nr.valid_until = r.valid_until,
        nr.created_at = r.created_at,
        nr.migrated_from = source.id
    ON MATCH SET
        nr.confidence = CASE
            WHEN coalesce(r.confidence, 0.0) > coalesce(nr.confidence, 0.0)
            THEN r.confidence ELSE nr.confidence END,
        nr.support = coalesce(nr.support, 1) + coalesce(r.support, 1),
        nr.derived = coalesce(nr.derived, false) AND coalesce(r.derived, false),
        nr.source_message_ids = (coalesce(nr.source_message_ids, []) +
            [x IN coalesce(r.source_message_ids, [])
             WHERE NOT x IN coalesce(nr.source_message_ids, [])])[..25],
        nr.evidence = (coalesce(nr.evidence, []) +
            [x IN coalesce(r.evidence, [])
             WHERE NOT x IN coalesce(nr.evidence, [])])[..3],
        nr.migrated_from = source.id,
        nr.updated_at = datetime()
    RETURN count(*) AS relatedOutTransferred
}
CALL (source, target, already_merged) {
    MATCH (other:Entity)-[r:RELATED_TO]->(source)
    WHERE other <> target AND NOT already_merged
    WITH source, target, other, r,
         coalesce(r.type, r.relation_type, 'RELATED_TO') AS rel_type
    MERGE (other)-[nr:RELATED_TO {type: rel_type}]->(target)
    ON CREATE SET
        nr.id = r.id,
        nr.relation_type = rel_type,
        nr.description = r.description,
        nr.confidence = r.confidence,
        nr.support = coalesce(r.support, 1),
        nr.derived = coalesce(r.derived, false),
        nr.extractor = r.extractor,
        nr.source_message_ids = coalesce(r.source_message_ids, []),
        nr.evidence = coalesce(r.evidence, []),
        nr.valid_from = r.valid_from,
        nr.valid_until = r.valid_until,
        nr.created_at = r.created_at,
        nr.migrated_from = source.id
    ON MATCH SET
        nr.confidence = CASE
            WHEN coalesce(r.confidence, 0.0) > coalesce(nr.confidence, 0.0)
            THEN r.confidence ELSE nr.confidence END,
        nr.support = coalesce(nr.support, 1) + coalesce(r.support, 1),
        nr.derived = coalesce(nr.derived, false) AND coalesce(r.derived, false),
        nr.source_message_ids = (coalesce(nr.source_message_ids, []) +
            [x IN coalesce(r.source_message_ids, [])
             WHERE NOT x IN coalesce(nr.source_message_ids, [])])[..25],
        nr.evidence = (coalesce(nr.evidence, []) +
            [x IN coalesce(r.evidence, [])
             WHERE NOT x IN coalesce(nr.evidence, [])])[..3],
        nr.migrated_from = source.id,
        nr.updated_at = datetime()
    RETURN count(*) AS relatedInTransferred
}
CALL (source, target) {
    MATCH (source)-[r:EXTRACTED_FROM]->(m:Message)
    WHERE NOT (target)-[:EXTRACTED_FROM]->(m)
    MERGE (target)-[nr:EXTRACTED_FROM]->(m)
    ON CREATE SET
        nr.confidence = r.confidence,
        nr.start_pos = r.start_pos,
        nr.end_pos = r.end_pos,
        nr.context = r.context,
        nr.created_at = r.created_at,
        nr.migrated_from = source.id
    RETURN count(*) AS extractedFromTransferred
}
CALL (source, target) {
    MATCH (source)-[r:EXTRACTED_BY]->(ex:Extractor)
    WHERE NOT (target)-[:EXTRACTED_BY]->(ex)
    MERGE (target)-[nr:EXTRACTED_BY]->(ex)
    ON CREATE SET
        nr.confidence = r.confidence,
        nr.extraction_time_ms = r.extraction_time_ms,
        nr.created_at = r.created_at,
        nr.migrated_from = source.id
    RETURN count(*) AS extractedByTransferred
}
CALL (source, target) {
    MATCH (p:Preference)-[r:APPLIES_TO]->(source)
    WHERE NOT (p)-[:APPLIES_TO]->(target)
    MERGE (p)-[nr:APPLIES_TO]->(target)
    ON CREATE SET nr.migrated_from = source.id
    RETURN count(*) AS appliesToTransferred
}
CALL (source, target) {
    MATCH (s:ReasoningStep)-[r:TOUCHED]->(source)
    WHERE NOT (s)-[:TOUCHED]->(target)
    MERGE (s)-[nr:TOUCHED]->(target)
    ON CREATE SET
        nr.recorded_at = r.recorded_at,
        nr.migrated_from = source.id
    RETURN count(*) AS touchedTransferred
}
SET source.merged_into = target.id,
    source.merged_at = CASE WHEN already_merged THEN source.merged_at ELSE datetime() END
SET target.aliases = CASE
    WHEN target.aliases IS NULL THEN [source.name]
    WHEN NOT source.name IN target.aliases THEN target.aliases + source.name
    ELSE target.aliases
END
";

pub const CREATE_SAME_AS_RELATIONSHIP: &str = "
MATCH (e1:Entity {id: $source_id})
MATCH (e2:Entity {id: $target_id})
WHERE NOT (e1)-[:SAME_AS]-(e2)
CREATE (e1)-[r:SAME_AS {
    confidence: $confidence,
    match_type: $match_type,
    created_at: datetime(),
    status: $status
}]->(e2)
RETURN r
";

/// The documented enrichment properties, and nothing else
/// (`enrichment/background.py::_update_entity` and the reference
/// `_ENRICHMENT_PROPERTY_MAP`).
///
/// `enriched_description`, `enriched_summary`, `enrichment_provider` and
/// `enriched_at` are replaced on every successful enrichment; the
/// provider-specific surfaces keep a previously stored value when the new
/// answer does not carry one, exactly like upstream's `coalesce`.
pub const SET_ENTITY_ENRICHMENT: &str = "
MATCH (e:Entity {id: $id})
SET e.enriched_description = $enriched_description,
    e.enriched_summary = $enriched_summary,
    e.enrichment_provider = $enrichment_provider,
    e.enriched_at = datetime($enriched_at),
    e.wikipedia_url = coalesce($wikipedia_url, e.wikipedia_url),
    e.wikidata_id = coalesce($wikidata_id, e.wikidata_id),
    e.diffbot_uri = coalesce($diffbot_uri, e.diffbot_uri),
    e.image_url = coalesce($image_url, e.image_url)
RETURN e.id AS id
";

/// `(:Entity).location` for a geocoded `LOCATION`, using the same
/// `point({latitude, longitude})` expression as [`create_entity_query`].
pub const SET_ENTITY_LOCATION: &str = "
MATCH (e:Entity {id: $id})
SET e.location = point({latitude: $latitude, longitude: $longitude})
RETURN e.id AS id
";

/// Redacted prerequisite error for the coordinate write. Naming the object and
/// the remedy is enough to act on; no credential, URL or payload can appear.
pub const LOCATION_INDEX_REQUIRED: &str = "entity location coordinates require the \
     entity_location_idx point index on (:Entity {location}); run schema setup; no schema was \
     changed";

/// Point index the coordinate write depends on (`schema.POINT_INDEXES`).
pub const ENTITY_LOCATION_INDEX_NAME: &str = "entity_location_idx";
pub const ENTITY_LOCATION_INDEX_LABEL: &str = "Entity";
pub const ENTITY_LOCATION_INDEX_PROPERTY: &str = "location";

/// Live-catalog definition of that index. Read-only: the writer verifies the
/// prerequisite and refuses the write when it is absent or unusable, and never
/// creates, repairs or drops schema.
pub const ENTITY_LOCATION_INDEX: &str = "
SHOW INDEXES YIELD name, type, entityType, labelsOrTypes, properties, state
WHERE name = $name
RETURN type, entityType, labelsOrTypes, properties, state
";

pub const UPDATE_SAME_AS_STATUS: &str = "
MATCH (e1:Entity {id: $source_id})-[r:SAME_AS]-(e2:Entity {id: $target_id})
SET r.status = $status, r.updated_at = datetime()
RETURN r
";

pub const LINK_MESSAGE_TO_ENTITY: &str = "
MATCH (m:Message {id: $message_id})
MATCH (e:Entity {id: $entity_id})
MERGE (m)-[r:MENTIONS]->(e)
ON CREATE SET
    r.confidence = $confidence,
    r.start_pos = $start_pos,
    r.end_pos = $end_pos,
    r.derived = $derived,
    r.extractor = $extractor,
    r.support = $support,
    r.source_message_ids = $source_message_ids,
    r.evidence = $evidence
RETURN r
";

/// Muninn: cross-conversation mentions (hosted `GET /entities/{id}/history`).
pub const GET_ENTITY_MENTIONS: &str = "
MATCH (e:Entity {id: $id})
OPTIONAL MATCH (c:Conversation)-[:HAS_MESSAGE]->(m:Message)-[:MENTIONS]->(e)
WITH e, c, m ORDER BY m.timestamp ASC
RETURN e.id AS entity_id, collect(CASE WHEN m IS NULL THEN NULL ELSE {
    conversation_id: c.id,
    message_id: m.id,
    content: m.content,
    timestamp: m.timestamp
} END) AS mentions
";

/// Muninn: entities and the `RELATED_TO` edges among them (hosted
/// `GET /entities/graph`). `SAME_AS` links carry no id upstream and are not
/// part of this view.
pub const GET_ENTITY_GRAPH: &str = "
MATCH (e:Entity)
WHERE e.merged_into IS NULL
WITH e ORDER BY e.created_at ASC LIMIT $limit
WITH collect(e) AS entities
CALL (entities) {
    UNWIND entities AS a
    MATCH (a)-[r:RELATED_TO]->(b:Entity)
    WHERE b IN entities AND r.id IS NOT NULL
    RETURN collect({
        id: r.id,
        source: a.id,
        target: b.id,
        type: coalesce(r.type, r.relation_type, type(r))
    }) AS edges
}
RETURN [x IN entities | {id: x.id, name: x.name, type: x.type}] AS nodes, edges
";

pub const CREATE_PREFERENCE: &str = "
CREATE (p:Preference {
    id: $id,
    category: $category,
    preference: $preference,
    context: $context,
    confidence: $confidence,
    embedding: $embedding,
    created_at: datetime(),
    metadata: $metadata
})
RETURN p
";

/// Bi-temporal start, written right after `CREATE_PREFERENCE` upstream.
pub const SET_PREFERENCE_VALID_FROM: &str = "
MATCH (p:Preference {id: $id}) SET p.valid_from = datetime()
RETURN p
";

pub const FIND_DUPLICATE_PREFERENCES: &str = "
CALL db.index.vector.queryNodes('preference_embedding_idx', $limit, $embedding)
YIELD node, score
WHERE score >= $threshold
  AND node.category = $category
  AND ($user_identifier IS NULL OR EXISTS { MATCH (:User {identifier: $user_identifier})-[:HAS_PREFERENCE]->(node) })
RETURN node AS p, score
ORDER BY score DESC
";

/// Upstream `_link_user_to_preference` (inline statement).
pub const LINK_USER_TO_PREFERENCE: &str = "
MERGE (u:User {identifier: $user_identifier})
ON CREATE SET u.id = $user_identifier, u.created_at = datetime()
WITH u
MATCH (p:Preference {id: $preference_id})
MERGE (u)-[:HAS_PREFERENCE]->(p)
";

/// Mark the old preference as superseded by the new one; idempotent.
pub const SUPERSEDE_PREFERENCE: &str = "
MATCH (old:Preference {id: $old_id})
MATCH (new:Preference {id: $new_id})
MERGE (old)-[:SUPERSEDED_BY]->(new)
SET old.valid_until = coalesce(old.valid_until, datetime())
";

/// Preferences linked to a user, most recent first; optionally hide superseded.
pub const PREFERENCES_FOR_USER: &str = "
MATCH (u:User {identifier: $user_identifier})-[:HAS_PREFERENCE]->(p:Preference)
WHERE ($active_only = false OR NOT (p)-[:SUPERSEDED_BY]->(:Preference))
  AND ($applies_to IS NULL OR EXISTS {
      MATCH (p)-[:APPLIES_TO]->(:Entity {id: $applies_to})
  })
  AND ($as_of IS NULL OR (
      (p.valid_from IS NULL OR p.valid_from <= datetime($as_of))
      AND (p.valid_until IS NULL OR p.valid_until > datetime($as_of))
  ))
RETURN DISTINCT p
ORDER BY p.created_at DESC
";

/// `(:Preference)-[:APPLIES_TO]->(:Entity)`, idempotent like upstream's
/// `_link_preference_to_entity`.
pub const LINK_PREFERENCE_TO_ENTITY: &str = "
MATCH (p:Preference {id: $preference_id})
MATCH (e:Entity {id: $entity_id})
MERGE (p)-[:APPLIES_TO]->(e)
";

pub const SEARCH_PREFERENCES_BY_EMBEDDING: &str = "
CALL db.index.vector.queryNodes('preference_embedding_idx', $limit, $embedding)
YIELD node, score
WHERE score >= $threshold
  AND ($user_identifier IS NULL OR EXISTS { MATCH (:User {identifier: $user_identifier})-[:HAS_PREFERENCE]->(node) })
RETURN node AS p, score
ORDER BY score DESC
";

pub const SEARCH_PREFERENCES_BY_CATEGORY: &str = "
MATCH (p:Preference {category: $category})
WHERE ($user_identifier IS NULL OR EXISTS { MATCH (:User {identifier: $user_identifier})-[:HAS_PREFERENCE]->(p) })
RETURN p
ORDER BY p.confidence DESC, p.created_at DESC
LIMIT $limit
";

pub const CREATE_FACT: &str = "
CREATE (f:Fact {
    id: $id,
    subject: $subject,
    predicate: $predicate,
    object: $object,
    confidence: $confidence,
    embedding: $embedding,
    valid_from: $valid_from,
    valid_until: $valid_until,
    created_at: datetime(),
    metadata: $metadata
})
RETURN f
";

pub const FIND_DUPLICATE_FACTS: &str = "
CALL db.index.vector.queryNodes('fact_embedding_idx', $limit, $embedding)
YIELD node, score
WHERE score >= $threshold
  AND node.subject = $subject
  AND node.predicate = $predicate
RETURN node AS f, score
ORDER BY score DESC
";

pub const UPDATE_FACT_CONFIDENCE: &str = "
MATCH (f:Fact {id: $id})
SET f.confidence = CASE
    WHEN $confidence > f.confidence THEN $confidence ELSE f.confidence END,
    f.object = CASE
    WHEN $confidence > f.confidence THEN $object ELSE f.object END
RETURN f
";

pub const SEARCH_FACTS_BY_EMBEDDING: &str = "
CALL db.index.vector.queryNodes('fact_embedding_idx', $limit, $embedding)
YIELD node, score
WHERE score >= $threshold
RETURN node AS f, score
ORDER BY score DESC
";

pub const GET_FACTS_BY_SUBJECT: &str = "
MATCH (f:Fact)
WHERE f.subject = $subject
RETURN f
ORDER BY f.confidence DESC, f.created_at DESC
LIMIT $limit
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_clause_matches_upstream() {
        assert_eq!(
            entity_keys_set_clause("e"),
            "e.name_key = toLower(toString(e.name)),\n    e.surface_keys = reduce(acc = '|', \
             form IN [e.name, e.canonical_name] + coalesce(e.aliases, []) | CASE WHEN form IS \
             NULL THEN acc ELSE acc + replace(toLower(toString(form)), '|', ' ') + '|' END)"
        );
    }

    #[test]
    fn labels_are_pascal_case_and_injection_safe() {
        assert_eq!(sanitize_label("PERSON").as_deref(), Some("Person"));
        assert_eq!(sanitize_label("my_type").as_deref(), Some("MyType"));
        assert_eq!(sanitize_label("Bad`) DETACH DELETE n //"), None);
        assert_eq!(sanitize_label("1abc"), None);
        assert_eq!(sanitize_label(""), None);
    }

    #[test]
    fn poleo_types_only_accept_known_subtypes() {
        assert_eq!(
            label_set_clause("OBJECT", Some("VEHICLE"), "e").as_deref(),
            Some("SET e:Object, e:Vehicle")
        );
        assert_eq!(
            label_set_clause("OBJECT", Some("SPACESHIP"), "e").as_deref(),
            Some("SET e:Object")
        );
        assert_eq!(
            label_set_clause("CUSTOM_KIND", Some("anything"), "e").as_deref(),
            Some("SET e:CustomKind, e:Anything")
        );
        assert_eq!(label_set_clause("not a label", None, "e"), None);
    }

    #[test]
    fn create_entity_query_sets_labels_keys_and_location() {
        let query = create_entity_query("LOCATION", Some("CITY"));
        assert!(query.starts_with("MERGE (e:Entity {name: $name, type: $type})"));
        assert!(query.contains("SET e:Location, e:City\nRETURN e"));
        assert!(query.contains("e.location = CASE WHEN $location IS NOT NULL"));
        assert!(query.contains("e.surface_keys = reduce("));
        assert!(!create_entity_query("PERSON", None).contains("e.location"));
    }

    #[test]
    fn depth_traversal_embeds_the_bound() {
        assert!(entity_relationships_within(3).contains("[:RELATED_TO*1..3]"));
    }
    #[test]
    fn preferences_scope_all_read_paths() {
        for query in [
            FIND_DUPLICATE_PREFERENCES,
            SEARCH_PREFERENCES_BY_EMBEDDING,
            SEARCH_PREFERENCES_BY_CATEGORY,
        ] {
            assert!(query.contains("$user_identifier IS NULL OR EXISTS"));
            assert!(query.contains("[:HAS_PREFERENCE]->"));
            assert!(query.contains("identifier: $user_identifier"));
        }
    }

    #[test]
    fn merge_mentions_preserves_properties_and_existing_positions() {
        assert!(MERGE_ENTITIES.contains("[original:MENTIONS]"));
        assert!(MERGE_ENTITIES.contains("ON CREATE SET mention = properties(original)"));
        assert!(MERGE_ENTITIES.contains("WHERE mention[key] IS NULL"));
        assert!(MERGE_ENTITIES.contains("SET mention[key] = original[key]"));
        assert!(MERGE_ENTITIES.contains("source_message_ids"));
        assert!(MERGE_ENTITIES.contains("source.merged_into = target.id"));
    }

    #[test]
    fn discovery_excludes_merged_but_id_lookup_keeps_history() {
        assert!(GET_ENTITY_BY_NAME.contains("WHERE e.merged_into IS NULL AND ("));
        assert!(GET_ENTITY_BY_NAME.contains("COALESCE(e.aliases, []))"));
        assert!(SEARCH_ENTITIES_BY_EMBEDDING.contains("node.merged_into IS NULL"));
        assert!(SEARCH_ENTITIES_BY_TYPE.contains("WHERE e.merged_into IS NULL"));
        assert!(GET_ENTITY_GRAPH.contains("WHERE e.merged_into IS NULL"));
        assert!(LIST_ENTITIES.contains("WHERE e.merged_into IS NULL"));
        assert!(!GET_ENTITY.contains("merged_into"));
    }

    /// The enrichment writer may touch exactly the documented property set and
    /// nothing else: no ontology label, embedding, alias, metadata blob, vector,
    /// credential or provider payload.
    #[test]
    fn set_entity_enrichment_writes_only_documented_properties() {
        const DOCUMENTED: [&str; 8] = [
            "e.enriched_description =",
            "e.enriched_summary =",
            "e.enrichment_provider =",
            "e.enriched_at = datetime($enriched_at)",
            "e.wikipedia_url = coalesce($wikipedia_url, e.wikipedia_url)",
            "e.wikidata_id = coalesce($wikidata_id, e.wikidata_id)",
            "e.diffbot_uri = coalesce($diffbot_uri, e.diffbot_uri)",
            "e.image_url = coalesce($image_url, e.image_url)",
        ];
        for property in DOCUMENTED {
            assert!(
                SET_ENTITY_ENRICHMENT.contains(property),
                "missing documented write: {property}"
            );
        }
        // One MATCH on identity, one SET list, one RETURN: no MERGE, no labels.
        assert_eq!(SET_ENTITY_ENRICHMENT.matches("SET ").count(), 1);
        assert!(SET_ENTITY_ENRICHMENT
            .trim_start()
            .starts_with("MATCH (e:Entity {id: $id})"));
        assert!(SET_ENTITY_ENRICHMENT
            .trim_end()
            .ends_with("RETURN e.id AS id"));
        assert_eq!(
            SET_ENTITY_ENRICHMENT.matches(" = ").count(),
            DOCUMENTED.len()
        );
        for forbidden in [
            "enrichment_data",
            "enrichment_metadata",
            "SET e:",
            "embedding",
            "aliases",
            "metadata",
            "created_at",
            "updated_at",
            "MATCH (n)",
        ] {
            assert!(
                !SET_ENTITY_ENRICHMENT.contains(forbidden),
                "unexpected write or scope: {forbidden}"
            );
        }
    }

    /// The coordinate write reuses the entity point pattern already used by
    /// `create_entity_query`, and performs no schema mutation.
    #[test]
    fn set_entity_location_uses_the_existing_point_pattern() {
        assert!(SET_ENTITY_LOCATION.contains("point({latitude: $latitude, longitude: $longitude})"));
        assert!(SET_ENTITY_LOCATION
            .trim_start()
            .starts_with("MATCH (e:Entity {id: $id})"));
        assert_eq!(SET_ENTITY_LOCATION.matches("SET ").count(), 1);
        for forbidden in ["CREATE INDEX", "DROP INDEX", "CREATE CONSTRAINT", "MERGE"] {
            assert!(!SET_ENTITY_LOCATION.contains(forbidden));
        }
        // Same point expression the entity upsert already uses for LOCATION.
        assert!(create_entity_query("LOCATION", None)
            .contains("point({latitude: $location.latitude, longitude: $location.longitude})"));
    }

    /// The prerequisite index is read from the live catalog; the writer never
    /// creates or repairs schema.
    #[test]
    fn entity_location_index_is_read_from_the_live_catalog() {
        assert!(ENTITY_LOCATION_INDEX.contains("SHOW INDEXES"));
        assert!(ENTITY_LOCATION_INDEX.contains("WHERE name = $name"));
        for column in ["type", "entityType", "labelsOrTypes", "properties", "state"] {
            assert!(
                ENTITY_LOCATION_INDEX.contains(column),
                "missing index column: {column}"
            );
        }
        assert_eq!(
            LOCATION_INDEX_REQUIRED,
            "entity location coordinates require the entity_location_idx point index on \
             (:Entity {location}); run schema setup; no schema was changed"
        );
        assert_eq!(ENTITY_LOCATION_INDEX_NAME, "entity_location_idx");
        assert_eq!(ENTITY_LOCATION_INDEX_LABEL, "Entity");
        assert_eq!(ENTITY_LOCATION_INDEX_PROPERTY, "location");
        for forbidden in ["CREATE", "DROP"] {
            assert!(!ENTITY_LOCATION_INDEX.contains(forbidden));
        }
    }
}

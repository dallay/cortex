//! Bolt-backed ontology store — `client.ontology` on your own Neo4j.
//!
//! `BoltOntology` implements the same surface the hosted backend exposes,
//! but writes directly to Neo4j instead of going through REST.

use std::collections::{HashMap, HashSet};

use muninn_core::{MemoryError, MemoryResult};
use muninn_neo4j::bolt::{self, Props};
use muninn_neo4j::client::Neo4jClient;
use neo4rs::BoltType;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use async_trait::async_trait;

use crate::builtin::{get_template, list_templates};
use crate::models::{
    ActiveOntology, ImportWarning, MigrationJob, Ontology, OntologyDiff, OntologyDocument,
    OntologyImportResult, OntologyRecord, OntologySummary, OntologyVersion,
};

/// Synthetic id prefix for the read-only built-in templates.
pub const TEMPLATE_ID_PREFIX: &str = "template:";

/// Default validation mode.
pub const DEFAULT_VALIDATION_MODE: &str = "permissive";

const VALIDATION_MODES: &[&str] = &["permissive", "strict"];
const LOCAL_IMPORT_FORMATS: &[&str] = &["auto", "json", "native"];

// -----------------------------------------------------------------------------
// Bolt param helpers
// -----------------------------------------------------------------------------

/// Convert an owned String to a BoltType::String.
fn bs(s: String) -> BoltType {
    BoltType::String(neo4rs::BoltString { value: s })
}

/// Convert a &str to a BoltType::String.
fn bs_ref(s: &str) -> BoltType {
    BoltType::String(neo4rs::BoltString {
        value: s.to_owned(),
    })
}

/// Build a param list from key-value pairs.
fn params<const N: usize>(pairs: [(&str, BoltType); N]) -> Vec<(&str, BoltType)> {
    pairs.to_vec()
}

// -----------------------------------------------------------------------------
// Cypher queries
// -----------------------------------------------------------------------------

const CREATE_ONTOLOGY: &str = r#"
CREATE (o:Ontology {
    id: $id,
    name: $name,
    description: $description,
    is_system: false,
    created_at: datetime()
})
RETURN o
"#;

const CREATE_ONTOLOGY_VERSION: &str = r#"
MATCH (o:Ontology {id: $ontology_id})
CREATE (o)-[:HAS_VERSION]->(v:OntologyVersion {
    id: $id,
    ontology_id: $ontology_id,
    revision: $revision,
    validation_mode: $validation_mode,
    document: $document,
    schema_hash: $schema_hash,
    is_active: false,
    created_at: datetime(),
    message: $message
})
RETURN v
"#;

const LIST_ONTOLOGIES: &str = r#"
MATCH (o:Ontology)
OPTIONAL MATCH (o)-[:HAS_VERSION]->(v:OntologyVersion)
WITH o,
     max(v.revision) AS current_revision,
     count(CASE WHEN v.is_active THEN 1 END) AS active_count
RETURN o AS ontology, current_revision, active_count > 0 AS is_active
ORDER BY o.created_at ASC, o.name ASC
"#;

const GET_ONTOLOGY: &str = r#"
MATCH (o:Ontology {id: $id})
OPTIONAL MATCH (o)-[:HAS_VERSION]->(v:OntologyVersion)
WITH o, v
ORDER BY v.revision ASC
RETURN o AS ontology, collect(v) AS versions
"#;

const GET_ONTOLOGY_VERSION: &str = r#"
MATCH (v:OntologyVersion {id: $id})
RETURN v
"#;

const GET_ONTOLOGY_MAX_REVISION: &str = r#"
MATCH (o:Ontology {id: $id})
OPTIONAL MATCH (o)-[:HAS_VERSION]->(v:OntologyVersion)
WITH o, v
ORDER BY v.revision DESC
WITH o, collect(v) AS versions
RETURN o.id AS ontology_id,
       coalesce(head(versions).revision, 0) AS max_revision,
       head(versions).validation_mode AS latest_validation_mode
"#;

const GET_ACTIVE_ONTOLOGY_VERSION: &str = r#"
MATCH (v:OntologyVersion)
WHERE v.is_active = true
WITH v
ORDER BY v.created_at DESC
LIMIT 1
OPTIONAL MATCH (o:Ontology {id: v.ontology_id})
RETURN v, o AS ontology
"#;

const ACTIVATE_ONTOLOGY_VERSION: &str = r#"
MATCH (v:OntologyVersion {id: $id})
MERGE (lock:OntologyLock {id: 'active'})
SET lock.updated_at = datetime()
WITH v
SET v.is_active = true
WITH v
OPTIONAL MATCH (other:OntologyVersion)
WHERE other.id <> v.id AND other.is_active = true
SET other.is_active = false
WITH v, count(other) AS deactivated
RETURN v, deactivated
"#;

const DELETE_ONTOLOGY: &str = r#"
MATCH (o:Ontology {id: $id})
OPTIONAL MATCH (o)-[:HAS_VERSION]->(v:OntologyVersion)
OPTIONAL MATCH (m:OntologyMigration {ontology_id: $id})
WITH o, collect(DISTINCT v) AS versions, collect(DISTINCT m) AS migrations
WITH size(versions) AS deleted_versions,
     size(migrations) AS deleted_migrations,
     versions + migrations + [o] AS doomed
UNWIND doomed AS node
DETACH DELETE node
RETURN deleted_versions, deleted_migrations
"#;

const CREATE_ONTOLOGY_MIGRATION: &str = r#"
CREATE (m:OntologyMigration {
    id: $id,
    ontology_id: $ontology_id,
    status: $status,
    total: $total,
    processed: $processed,
    errored: $errored,
    spec: $spec,
    error_message: $error_message,
    created_at: datetime(),
    completed_at: datetime()
})
RETURN m
"#;

const GET_ONTOLOGY_MIGRATION: &str = r#"
MATCH (m:OntologyMigration {id: $id})
RETURN m
"#;

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

fn document_json(document: &OntologyDocument) -> String {
    serde_json::to_string(document).unwrap_or_default()
}

fn schema_hash(document: &OntologyDocument) -> String {
    let key = document.content_key();
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn require_sound(document: &OntologyDocument) -> MemoryResult<()> {
    let problems = document.validate_structure();
    if problems.is_empty() {
        Ok(())
    } else {
        let listing = problems
            .iter()
            .map(|p| format!("  - {p}"))
            .collect::<Vec<_>>()
            .join("\n");
        Err(MemoryError::invalid(format!(
            "Ontology is structurally invalid:\n{listing}"
        )))
    }
}

fn check_validation_mode(value: Option<&str>) -> MemoryResult<Option<String>> {
    match value {
        None => Ok(None),
        Some(mode) => {
            let normalized = mode.trim().to_lowercase();
            if VALIDATION_MODES.contains(&normalized.as_str()) {
                Ok(Some(normalized))
            } else {
                Err(MemoryError::invalid(format!(
                    "validation_mode must be one of {}; got {mode:?}",
                    VALIDATION_MODES.join(", ")
                )))
            }
        }
    }
}

fn reject_template(ontology_id: &str, verb: &str) -> MemoryResult<()> {
    if ontology_id.starts_with(TEMPLATE_ID_PREFIX) {
        Err(MemoryError::invalid(format!(
            "Built-in template {ontology_id:?} is read-only and cannot be {verb}. \
             Clone it first: client.ontology.clone(<name>)."
        )))
    } else {
        Ok(())
    }
}

fn parse_stored_document(raw: &str) -> Option<OntologyDocument> {
    let val: serde_json::Value = serde_json::from_str(raw).ok()?;
    serde_json::from_value(val).ok()
}

fn version_from_props(props: &Props) -> OntologyVersion {
    let document_raw = props.string("document");
    let document = document_raw.as_deref().and_then(parse_stored_document);

    OntologyVersion {
        id: props.string("id").unwrap_or_default(),
        ontology_id: props.string("ontology_id").unwrap_or_default(),
        revision: props.i64("revision").unwrap_or(1),
        validation_mode: props
            .string("validation_mode")
            .unwrap_or_else(|| DEFAULT_VALIDATION_MODE.to_owned()),
        document,
        schema_hash: props.string("schema_hash"),
        created_at: props.string("created_at"),
        message: props.string("message"),
    }
}

fn migration_from_props(props: &Props) -> MigrationJob {
    let raw_spec = props.string("spec");
    let spec = raw_spec.and_then(|s| serde_json::from_str(&s).ok());

    MigrationJob {
        id: props.string("id").unwrap_or_default(),
        ontology_id: props.string("ontology_id"),
        workspace_id: None,
        status: props.string("status"),
        total: props.i64("total"),
        processed: props.i64("processed"),
        errored: props.i64("errored"),
        error_message: props.string("error_message"),
        spec,
        created_at: props.string("created_at"),
        updated_at: None,
        started_at: None,
        completed_at: props.string("completed_at"),
        created_by: None,
    }
}

fn record_from_props(props: &Props) -> OntologyRecord {
    OntologyRecord {
        id: props.string("id").unwrap_or_default(),
        name: props.string("name").unwrap_or_default(),
        description: props.string("description"),
        workspace_id: None,
        is_system: props.bool("is_system").unwrap_or_default(),
        created_at: props.string("created_at"),
    }
}

// -----------------------------------------------------------------------------
// Migration planning
// -----------------------------------------------------------------------------

struct MappingStep {
    from_label: String,
    to_label: String,
    subtype: Option<String>,
    pole_type: String,
    set_type: bool,
    type_label: Option<String>,
    subtype_label: Option<String>,
}

fn ontology_node_label(label: &str) -> Option<String> {
    if label.is_empty() {
        return None;
    }
    let chars: Vec<char> = label.chars().collect();
    if !chars[0].is_ascii_alphabetic() {
        return None;
    }
    for c in &chars[1..] {
        if !c.is_ascii_alphanumeric() && *c != '_' {
            return None;
        }
    }
    Some(label.to_string())
}

fn validate_entity_type(pole_type: &str) -> Option<String> {
    match pole_type {
        "PERSON" => Some("Person".to_owned()),
        "ORGANIZATION" => Some("Organization".to_owned()),
        "LOCATION" => Some("Location".to_owned()),
        "EVENT" => Some("Event".to_owned()),
        "OBJECT" => Some("Object".to_owned()),
        _ => None,
    }
}

fn plan_mapping(
    source: &OntologyDocument,
    target: &OntologyDocument,
    pair: (&str, &str),
) -> MemoryResult<MappingStep> {
    let (raw_from, raw_to) = pair;
    let from_label = ontology_node_label(raw_from).ok_or_else(|| {
        MemoryError::invalid(format!(
            "Type mapping {raw_from:?} contains a label that is not a valid Neo4j identifier"
        ))
    })?;
    let to_label = ontology_node_label(raw_to).ok_or_else(|| {
        MemoryError::invalid(format!(
            "Type mapping {raw_to:?} contains a label that is not a valid Neo4j identifier"
        ))
    })?;

    let target_type = target.entity_type(raw_to).ok_or_else(|| {
        MemoryError::invalid(format!(
            "Target label {raw_to:?} is not declared by the target ontology version; \
             declared labels: {}.",
            target.labels().join(", ")
        ))
    })?;

    let source_type = source.entity_type(raw_from);
    let pole_type = target_type.pole_type.to_uppercase();
    let set_type = source_type
        .map(|st| st.pole_type.to_uppercase() != pole_type)
        .unwrap_or(true);
    let subtype = target_type.subtype.as_deref().map(|s| s.to_uppercase());
    // The subtype becomes a Cypher label in the migration query, so it must be a
    // valid Neo4j identifier before any query runs.
    let subtype_label = match &subtype {
        Some(raw) => Some(ontology_node_label(raw).ok_or_else(|| {
            MemoryError::invalid(format!(
                "Type mapping {raw_to:?} contains a subtype {raw:?} that is not a valid Neo4j identifier"
            ))
        })?),
        None => None,
    };

    Ok(MappingStep {
        from_label,
        to_label,
        subtype: subtype_label.clone(),
        pole_type: pole_type.clone(),
        set_type,
        type_label: validate_entity_type(&pole_type),
        subtype_label,
    })
}

fn count_entities_with_label_query(label: &str) -> String {
    format!(
        r#"
MATCH (e:{label})
RETURN count(e) AS matched
"#
    )
}

fn relabel_entities_query(
    from_label: &str,
    to_label: &str,
    set_type: bool,
    type_label: Option<&str>,
    subtype_label: Option<&str>,
) -> String {
    let mut clauses = Vec::new();
    clauses.push(format!("REMOVE e:`{from_label}`"));
    clauses.push(format!("SET e:`{to_label}`"));

    if let Some(sub) = subtype_label {
        clauses.push(format!("SET e:`{sub}`, e.subtype = $subtype"));
    }

    if set_type {
        if let Some(tl) = type_label {
            clauses.push(format!("SET e:`{tl}`, e.type = $type"));
        }
    }

    let actions = clauses.join("\n    ");
    format!(
        r#"
MATCH (e:`{from_label}`)
    {actions}
RETURN count(e) AS migrated
"#
    )
}

// -----------------------------------------------------------------------------
// Inline parsing (import_)
// -----------------------------------------------------------------------------

fn parse_inline(content: &str, _requested: &str) -> MemoryResult<serde_json::Value> {
    let data: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| MemoryError::invalid(format!("Not valid JSON: {e}")))?;

    if data.is_object() {
        Ok(data)
    } else {
        Err(MemoryError::invalid(
            "Ontology content does not decode to a mapping.",
        ))
    }
}

// -----------------------------------------------------------------------------
// BoltOntology
// -----------------------------------------------------------------------------

/// `client.ontology` on bolt — versioned ontologies stored in Neo4j.
pub struct BoltOntology {
    client: Neo4jClient,
}

impl BoltOntology {
    pub fn new(client: Neo4jClient) -> Self {
        Self { client }
    }

    // -- reads ----------------------------------------------------------------

    pub async fn list(&self) -> MemoryResult<Vec<OntologySummary>> {
        let mut summaries: Vec<OntologySummary> = list_templates()
            .iter()
            .map(|name| self.template_summary(name))
            .collect();

        let rows = self.client.execute(LIST_ONTOLOGIES, Vec::new()).await?;
        for row in &rows {
            let ontology_bolt = bolt::column(row, "ontology")?;
            let props = Props::of(&ontology_bolt)?;

            let id = props.string("id").unwrap_or_default();
            let name = props.string("name").unwrap_or_default();
            let description = props.string("description");
            let current_revision = props.i64("current_revision");
            let is_active = props.bool("is_active").unwrap_or_default();

            summaries.push(OntologySummary {
                id: id.clone(),
                name,
                display_name: Some(id),
                description,
                emoji: None,
                tagline: None,
                is_system: false,
                current_revision,
                is_active,
            });
        }
        Ok(summaries)
    }

    pub async fn get(&self, ontology_id: &str) -> MemoryResult<Ontology> {
        if ontology_id.starts_with(TEMPLATE_ID_PREFIX) {
            return self.template_ontology(ontology_id);
        }

        let rows = self
            .client
            .execute(GET_ONTOLOGY, params([("id", bs_ref(ontology_id))]))
            .await?;

        if rows.is_empty() {
            return Err(MemoryError::not_found(format!(
                "Ontology {ontology_id:?} not found."
            )));
        }

        let row = &rows[0];
        let ontology_bolt = bolt::column(row, "ontology")?;
        let oprops = Props::of(&ontology_bolt)?;
        let record = record_from_props(&oprops);

        let versions_bolt: Option<BoltType> = row.get("versions").ok();
        let mut versions = Vec::new();
        if let Some(BoltType::List(list)) = versions_bolt {
            for item in &list.value {
                if let Ok(props) = Props::of(item) {
                    versions.push(version_from_props(&props));
                }
            }
        }
        versions.sort_by_key(|v| v.revision);

        Ok(Ontology { record, versions })
    }

    pub async fn get_active(&self) -> MemoryResult<ActiveOntology> {
        let rows = self
            .client
            .execute(GET_ACTIVE_ONTOLOGY_VERSION, Vec::new())
            .await?;

        if rows.is_empty() {
            return Err(MemoryError::NotSupported(
                "No active ontology bound for this database. \
                 Create or clone one, then call client.ontology.activate(version.id)."
                    .to_owned(),
            ));
        }

        let row = &rows[0];
        let v_bolt = bolt::column(row, "v")?;
        let vprops = Props::of(&v_bolt)?;
        let version = version_from_props(&vprops);

        let document = version.document.ok_or_else(|| {
            MemoryError::invalid(format!(
                "Active ontology version {:?} holds an unreadable document; \
                 activate a different revision or re-create it.",
                version.id
            ))
        })?;

        let ontology_id_from_parent: Option<String> = (|| {
            let bolt_val = bolt::column(row, "ontology").ok()?;
            let props = Props::of(&bolt_val).ok()?;
            props.string("id")
        })();

        let ontology_id = if !version.ontology_id.is_empty() {
            Some(version.ontology_id.clone())
        } else {
            ontology_id_from_parent
        };

        Ok(ActiveOntology {
            document,
            validation_mode: Some(version.validation_mode),
            revision: Some(version.revision),
            ontology_id,
            version_id: Some(version.id),
            schema_hash: version.schema_hash,
        })
    }

    // -- writes ---------------------------------------------------------------

    pub async fn clone(&self, template_name: &str) -> MemoryResult<OntologyVersion> {
        let template = get_template(template_name);
        let name = self.unique_name(template_name).await?;
        let mut document = template.clone();
        document.domain.id = name.clone();
        document.domain.name = name.clone();

        self.insert(
            &name,
            &document,
            DEFAULT_VALIDATION_MODE,
            Some(&format!("Cloned from built-in template {template_name:?}")),
        )
        .await
    }

    pub async fn create(
        &self,
        name: &str,
        document: &OntologyDocument,
        validation_mode: Option<&str>,
    ) -> MemoryResult<OntologyVersion> {
        let mode = check_validation_mode(validation_mode)?
            .unwrap_or_else(|| DEFAULT_VALIDATION_MODE.to_owned());
        require_sound(document)?;

        let final_name = if !document.domain.id.is_empty() {
            document.domain.id.clone()
        } else if !document.domain.name.is_empty() {
            document.domain.name.clone()
        } else {
            name.to_owned()
        };

        self.insert(&final_name, document, &mode, None).await
    }

    pub async fn update(
        &self,
        ontology_id: &str,
        document: &OntologyDocument,
        validation_mode: Option<&str>,
    ) -> MemoryResult<OntologyVersion> {
        reject_template(ontology_id, "updated")?;
        let mode = check_validation_mode(validation_mode);
        require_sound(document)?;

        let rows = self
            .client
            .execute(
                GET_ONTOLOGY_MAX_REVISION,
                params([("id", bs_ref(ontology_id))]),
            )
            .await?;

        if rows.is_empty() {
            return Err(MemoryError::not_found(format!(
                "Ontology {ontology_id:?} not found."
            )));
        }

        let row = &rows[0];
        let max_revision: i64 = row.get("max_revision").unwrap_or(0);
        let latest_mode: Option<String> = row.get("latest_validation_mode").ok();

        let effective_mode = mode?
            .unwrap_or_else(|| latest_mode.unwrap_or_else(|| DEFAULT_VALIDATION_MODE.to_owned()));

        self.add_version(
            ontology_id,
            document,
            max_revision + 1,
            &effective_mode,
            None,
        )
        .await
    }

    pub async fn activate(&self, version_id: &str) -> MemoryResult<OntologyVersion> {
        let rows = self
            .client
            .execute(
                ACTIVATE_ONTOLOGY_VERSION,
                params([("id", bs_ref(version_id))]),
            )
            .await?;

        if rows.is_empty() {
            return Err(MemoryError::not_found(format!(
                "Ontology version {version_id:?} not found."
            )));
        }

        let row = &rows[0];
        let v_bolt = bolt::column(row, "v")?;
        let props = Props::of(&v_bolt)?;
        Ok(version_from_props(&props))
    }

    pub async fn delete(&self, ontology_id: &str) -> MemoryResult<()> {
        reject_template(ontology_id, "deleted")?;

        let rows = self
            .client
            .execute(DELETE_ONTOLOGY, params([("id", bs_ref(ontology_id))]))
            .await?;

        if rows.is_empty() {
            return Err(MemoryError::not_found(format!(
                "Ontology {ontology_id:?} not found."
            )));
        }

        Ok(())
    }

    // -- conversion + comparison ---------------------------------------------

    pub async fn import_(
        &self,
        content: Option<&str>,
        url: Option<&str>,
        format: Option<&str>,
    ) -> MemoryResult<OntologyImportResult> {
        if url.is_some() {
            return Err(MemoryError::NotSupported(
                "Fetching an ontology by URL happens server-side on NAMS; \
                 the bolt store only converts content you hand it."
                    .to_owned(),
            ));
        }

        let content = content.ok_or_else(|| {
            MemoryError::invalid("import_ requires content (url is a NAMS capability).")
        })?;

        let requested = (format.unwrap_or("auto")).to_lowercase();
        if !LOCAL_IMPORT_FORMATS.contains(&requested.as_str()) {
            return Err(MemoryError::NotSupported(format!(
                "Only {} are converted locally; extraction-backed converters \
                 (rdf, graphql, cypher, linkml, ...) are a NAMS capability.",
                LOCAL_IMPORT_FORMATS.join(", ")
            )));
        }

        let mut warnings = Vec::new();
        let data = parse_inline(content, &requested)?;
        let doc: OntologyDocument = serde_json::from_value(data.clone())
            .map_err(|e| MemoryError::invalid(format!("Ontology document is not valid: {e}")))?;

        warnings.extend(
            doc.validate_structure()
                .into_iter()
                .map(|problem| ImportWarning {
                    code: Some("invalid_structure".to_owned()),
                    message: Some(problem),
                    path: None,
                }),
        );

        Ok(OntologyImportResult {
            document: Some(doc),
            warnings,
            detected_format: Some("native".to_owned()),
            suggested_name: None,
        })
    }

    pub async fn diff(
        &self,
        ontology_id: &str,
        from_revision: i64,
        to_revision: i64,
    ) -> MemoryResult<OntologyDiff> {
        let ontology = self.get(ontology_id).await?;
        compute_diff(&ontology, from_revision, to_revision)
    }

    // -- migration ------------------------------------------------------------

    pub async fn migrate(
        &self,
        ontology_id: &str,
        from_version_id: &str,
        to_version_id: &str,
        type_mappings: &[(String, String)],
        dry_run: bool,
    ) -> MemoryResult<MigrationJob> {
        if type_mappings.is_empty() {
            return Err(MemoryError::invalid(
                "migrate() requires at least one type mapping.",
            ));
        }

        let source_document = self.version_document(from_version_id).await?;
        let target_document = self.version_document(to_version_id).await?;

        let plan: MemoryResult<Vec<MappingStep>> = type_mappings
            .iter()
            .map(|(from, to)| plan_mapping(&source_document, &target_document, (from, to)))
            .collect();
        let plan = plan?;

        let mut total: i64 = 0;
        let mut processed: i64 = 0;
        let mut errored: i64 = 0;
        let mut errors: Vec<String> = Vec::new();

        for step in &plan {
            let step_result: MemoryResult<()> = if dry_run {
                let rows = self
                    .client
                    .execute(
                        &count_entities_with_label_query(&step.from_label),
                        Vec::new(),
                    )
                    .await?;
                if let Some(row) = rows.first() {
                    let matched: i64 = row.get("matched").unwrap_or(0);
                    total += matched;
                }
                Ok(())
            } else {
                let query = relabel_entities_query(
                    &step.from_label,
                    &step.to_label,
                    step.set_type,
                    step.type_label.as_deref(),
                    step.subtype_label.as_deref(),
                );
                let mut p = Vec::new();
                if let Some(ref subtype) = step.subtype {
                    p.push(("subtype", bs(subtype.clone())));
                }
                if step.set_type {
                    p.push(("type", bs(step.pole_type.clone())));
                }
                let rows = self.client.execute(&query, p).await?;
                if let Some(row) = rows.first() {
                    let migrated: i64 = row.get("migrated").unwrap_or(0);
                    total += migrated;
                    processed += migrated;
                }
                Ok(())
            };

            if let Err(e) = step_result {
                errored += 1;
                errors.push(format!("{} -> {}: {}", step.from_label, step.to_label, e));
                tracing::warn!(
                    "Ontology migration step {} -> {} failed.",
                    step.from_label,
                    step.to_label
                );
            }
        }

        let spec = serde_json::json!({
            "from_version_id": from_version_id,
            "to_version_id": to_version_id,
            "type_mappings": type_mappings.iter().map(|(f, t)| {
                serde_json::json!({ "from": f, "to": t })
            }).collect::<Vec<_>>(),
            "dry_run": dry_run,
        });

        let job_id = Uuid::new_v4().to_string();
        let status = if errored > 0 { "failed" } else { "completed" };
        let error_message = if errors.is_empty() {
            None
        } else {
            Some(errors.join("; "))
        };

        let rows = self
            .client
            .execute(
                CREATE_ONTOLOGY_MIGRATION,
                vec![
                    ("id", bs(job_id.clone())),
                    ("ontology_id", bs_ref(ontology_id)),
                    ("status", bs_ref(status)),
                    ("total", BoltType::from(total)),
                    ("processed", BoltType::from(processed)),
                    ("errored", BoltType::from(errored)),
                    ("spec", bs(serde_json::to_string(&spec).unwrap_or_default())),
                    (
                        "error_message",
                        bs(error_message.as_deref().unwrap_or("").to_owned()),
                    ),
                ],
            )
            .await?;

        if rows.is_empty() {
            return Err(MemoryError::storage(
                "Failed to record the ontology migration job.",
            ));
        }

        let row = &rows[0];
        let m_bolt = bolt::column(row, "m")?;
        let mprops = Props::of(&m_bolt)?;
        let mut job = migration_from_props(&mprops);
        job.id = job_id;
        job.status = Some(status.to_owned());
        job.total = Some(total);
        job.processed = Some(processed);
        job.errored = Some(errored);
        job.error_message = error_message;
        job.spec = Some(spec);
        Ok(job)
    }

    pub async fn get_migration(&self, job_id: &str) -> MemoryResult<MigrationJob> {
        let rows = self
            .client
            .execute(GET_ONTOLOGY_MIGRATION, params([("id", bs_ref(job_id))]))
            .await?;

        if rows.is_empty() {
            return Err(MemoryError::not_found(format!(
                "Ontology migration {job_id:?} not found."
            )));
        }

        let row = &rows[0];
        let m_bolt = bolt::column(row, "m")?;
        let mprops = Props::of(&m_bolt)?;
        Ok(migration_from_props(&mprops))
    }

    // -- internals ------------------------------------------------------------

    fn template_summary(&self, name: &str) -> OntologySummary {
        template_summary(name)
    }

    fn template_ontology(&self, ontology_id: &str) -> MemoryResult<Ontology> {
        template_ontology(ontology_id)
    }

    async fn unique_name(&self, base: &str) -> MemoryResult<String> {
        let rows = self.client.execute(LIST_ONTOLOGIES, Vec::new()).await?;
        let mut taken = HashSet::new();
        for row in &rows {
            if let Ok(bolt_val) = row.get::<BoltType>("ontology") {
                if let Ok(props) = Props::of(&bolt_val) {
                    if let Some(name) = props.string("name") {
                        taken.insert(name);
                    }
                }
            }
        }
        if !taken.contains(base) {
            return Ok(base.to_owned());
        }
        let mut suffix = 1;
        while taken.contains(&format!("{base}-copy-{suffix}")) {
            suffix += 1;
        }
        Ok(format!("{base}-copy-{suffix}"))
    }

    async fn insert(
        &self,
        name: &str,
        document: &OntologyDocument,
        validation_mode: &str,
        message: Option<&str>,
    ) -> MemoryResult<OntologyVersion> {
        let ontology_id = Uuid::new_v4().to_string();
        let description = document.domain.description.clone().unwrap_or_default();

        self.client
            .execute(
                CREATE_ONTOLOGY,
                vec![
                    ("id", bs(ontology_id.clone())),
                    ("name", bs_ref(name)),
                    ("description", bs(description)),
                ],
            )
            .await?;

        self.add_version(&ontology_id, document, 1, validation_mode, message)
            .await
    }

    async fn add_version(
        &self,
        ontology_id: &str,
        document: &OntologyDocument,
        revision: i64,
        validation_mode: &str,
        message: Option<&str>,
    ) -> MemoryResult<OntologyVersion> {
        let id = Uuid::new_v4().to_string();
        let doc_json = document_json(document);
        let hash = schema_hash(document);

        let rows = self
            .client
            .execute(
                CREATE_ONTOLOGY_VERSION,
                vec![
                    ("id", bs(id)),
                    ("ontology_id", bs_ref(ontology_id)),
                    ("revision", BoltType::from(revision)),
                    ("validation_mode", bs_ref(validation_mode)),
                    ("document", bs(doc_json)),
                    ("schema_hash", bs(hash)),
                    ("message", bs(message.unwrap_or("").to_owned())),
                ],
            )
            .await?;

        if rows.is_empty() {
            return Err(MemoryError::not_found(format!(
                "Ontology {ontology_id:?} not found."
            )));
        }

        let row = &rows[0];
        let v_bolt = bolt::column(row, "v")?;
        let props = Props::of(&v_bolt)?;
        Ok(version_from_props(&props))
    }

    async fn version_document(&self, version_id: &str) -> MemoryResult<OntologyDocument> {
        let rows = self
            .client
            .execute(GET_ONTOLOGY_VERSION, params([("id", bs_ref(version_id))]))
            .await?;

        if rows.is_empty() {
            return Err(MemoryError::not_found(format!(
                "Ontology version {version_id:?} not found."
            )));
        }

        let row = &rows[0];
        let v_bolt = bolt::column(row, "v")?;
        let props = Props::of(&v_bolt)?;
        let version = version_from_props(&props);
        version.document.ok_or_else(|| {
            MemoryError::invalid(format!(
                "Ontology version {version_id:?} holds an unreadable document."
            ))
        })
    }
}

// -----------------------------------------------------------------------------
// OntologyApi impl
// -----------------------------------------------------------------------------

#[async_trait]
impl crate::protocol::OntologyApi for BoltOntology {
    async fn list(&self) -> MemoryResult<Vec<OntologySummary>> {
        self.list().await
    }

    async fn get(&self, ontology_id: &str) -> MemoryResult<Ontology> {
        self.get(ontology_id).await
    }

    async fn get_active(&self) -> MemoryResult<ActiveOntology> {
        self.get_active().await
    }

    async fn clone(&self, template_name: &str) -> MemoryResult<OntologyVersion> {
        self.clone(template_name).await
    }

    async fn create(
        &self,
        name: &str,
        document: &OntologyDocument,
        validation_mode: Option<&str>,
    ) -> MemoryResult<OntologyVersion> {
        self.create(name, document, validation_mode).await
    }

    async fn update(
        &self,
        ontology_id: &str,
        document: &OntologyDocument,
        validation_mode: Option<&str>,
    ) -> MemoryResult<OntologyVersion> {
        self.update(ontology_id, document, validation_mode).await
    }

    async fn activate(&self, version_id: &str) -> MemoryResult<OntologyVersion> {
        self.activate(version_id).await
    }

    async fn delete(&self, ontology_id: &str) -> MemoryResult<()> {
        self.delete(ontology_id).await
    }

    async fn import_(
        &self,
        content: Option<&str>,
        url: Option<&str>,
        format: Option<&str>,
    ) -> MemoryResult<OntologyImportResult> {
        self.import_(content, url, format).await
    }

    async fn diff(
        &self,
        ontology_id: &str,
        from_revision: i64,
        to_revision: i64,
    ) -> MemoryResult<OntologyDiff> {
        self.diff(ontology_id, from_revision, to_revision).await
    }

    async fn migrate(
        &self,
        ontology_id: &str,
        from_version_id: &str,
        to_version_id: &str,
        type_mappings: &[(String, String)],
        dry_run: bool,
    ) -> MemoryResult<MigrationJob> {
        self.migrate(
            ontology_id,
            from_version_id,
            to_version_id,
            type_mappings,
            dry_run,
        )
        .await
    }

    async fn get_migration(&self, job_id: &str) -> MemoryResult<MigrationJob> {
        self.get_migration(job_id).await
    }
}

/// Pull one revision's document out of a loaded ontology.
fn document_at(ontology: &Ontology, revision: i64) -> MemoryResult<OntologyDocument> {
    for version in &ontology.versions {
        if version.revision == revision {
            return version.document.clone().ok_or_else(|| {
                MemoryError::invalid(format!(
                    "Revision {revision} of ontology {:?} holds an unreadable document.",
                    ontology.record.id
                ))
            });
        }
    }
    Err(MemoryError::not_found(format!(
        "Ontology {:?} has no revision {revision}.",
        ontology.record.id
    )))
}

/// Compute differences between two revisions of an ontology.
fn compute_diff(
    ontology: &Ontology,
    from_revision: i64,
    to_revision: i64,
) -> MemoryResult<OntologyDiff> {
    let before = document_at(ontology, from_revision)?;
    let after = document_at(ontology, to_revision)?;

    let mut diff = OntologyDiff {
        from_revision: Some(from_revision),
        to_revision: Some(to_revision),
        entity_types: serde_json::Value::Object(serde_json::Map::new()),
        relationships: serde_json::Value::Object(serde_json::Map::new()),
        mode_change: None,
    };

    let before_labels: HashSet<&str> = before.labels().into_iter().collect();
    let after_labels: HashSet<&str> = after.labels().into_iter().collect();

    let added: Vec<&str> = after_labels.difference(&before_labels).copied().collect();
    let removed: Vec<&str> = before_labels.difference(&after_labels).copied().collect();

    let mut entity_changes = serde_json::Map::new();
    if !added.is_empty() {
        entity_changes.insert(
            "added".to_string(),
            serde_json::Value::Array(added.into_iter().map(|s| serde_json::json!(*s)).collect()),
        );
    }
    if !removed.is_empty() {
        entity_changes.insert(
            "removed".to_string(),
            serde_json::Value::Array(removed.into_iter().map(|s| serde_json::json!(*s)).collect()),
        );
    }
    diff.entity_types = serde_json::Value::Object(entity_changes);

    let before_patterns = before.patterns();
    let after_patterns = after.patterns();

    let rel_added: Vec<_> = after_patterns.difference(&before_patterns).collect();
    let rel_removed: Vec<_> = before_patterns.difference(&after_patterns).collect();

    let mut rel_changes = serde_json::Map::new();
    if !rel_added.is_empty() {
        rel_changes.insert(
            "added".to_string(),
            serde_json::Value::Array(
                rel_added
                    .iter()
                    .map(|(s, r, t)| serde_json::json!({ "source": s, "type": r, "target": t }))
                    .collect(),
            ),
        );
    }
    if !rel_removed.is_empty() {
        rel_changes.insert(
            "removed".to_string(),
            serde_json::Value::Array(
                rel_removed
                    .iter()
                    .map(|(s, r, t)| serde_json::json!({ "source": s, "type": r, "target": t }))
                    .collect(),
            ),
        );
    }
    diff.relationships = serde_json::Value::Object(rel_changes);

    let modes: HashMap<i64, String> = ontology
        .versions
        .iter()
        .map(|v| (v.revision, v.validation_mode.clone()))
        .collect();

    if let (Some(old_mode), Some(new_mode)) = (modes.get(&from_revision), modes.get(&to_revision)) {
        if old_mode != new_mode {
            diff.mode_change = Some(serde_json::json!({
                "from": old_mode,
                "to": new_mode
            }));
        }
    }

    Ok(diff)
}

/// Build the summary of a built-in ontology template.
///
/// # Panics
///
/// Panics if `name` is not a built-in template; see [`get_template`].
fn template_summary(name: &str) -> OntologySummary {
    let document = get_template(name);
    OntologySummary {
        id: format!("{TEMPLATE_ID_PREFIX}{name}"),
        name: name.to_owned(),
        display_name: Some(document.domain.name.clone()),
        description: document.domain.description.clone(),
        emoji: document.domain.emoji.clone(),
        tagline: document.domain.tagline.clone(),
        is_system: true,
        current_revision: None,
        is_active: false,
    }
}

/// Build the ontology of a built-in template id such as `template:poleo`.
///
/// # Panics
///
/// Panics if the id does not name a built-in template; see [`get_template`].
fn template_ontology(ontology_id: &str) -> MemoryResult<Ontology> {
    let name = ontology_id
        .strip_prefix(TEMPLATE_ID_PREFIX)
        .unwrap_or(ontology_id);
    let document = get_template(name);

    Ok(Ontology {
        record: OntologyRecord {
            id: ontology_id.to_owned(),
            name: name.to_owned(),
            description: document.domain.description.clone(),
            workspace_id: None,
            is_system: true,
            created_at: None,
        },
        versions: vec![OntologyVersion {
            id: format!("{ontology_id}:1"),
            ontology_id: ontology_id.to_owned(),
            revision: 1,
            validation_mode: DEFAULT_VALIDATION_MODE.to_owned(),
            document: Some(document),
            schema_hash: None,
            created_at: None,
            message: None,
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_id_prefix_is_correct() {
        assert_eq!(TEMPLATE_ID_PREFIX, "template:");
    }

    #[test]
    fn default_validation_mode_is_permissive() {
        assert_eq!(DEFAULT_VALIDATION_MODE, "permissive");
    }

    #[test]
    fn check_validation_mode_accepts_valid_modes() {
        assert!(check_validation_mode(Some("permissive")).is_ok());
        assert!(check_validation_mode(Some("strict")).is_ok());
        assert!(check_validation_mode(Some("Permissive")).is_ok());
        assert!(check_validation_mode(Some("STRICT")).is_ok());
        assert!(check_validation_mode(None).is_ok());
    }

    #[test]
    fn check_validation_mode_rejects_invalid() {
        assert!(check_validation_mode(Some("invalid")).is_err());
    }

    #[test]
    fn reject_template_blocks_mutation() {
        assert!(reject_template("template:podcast", "deleted").is_err());
        assert!(reject_template("template:podcast", "updated").is_err());
    }

    #[test]
    fn reject_template_allows_normal_ids() {
        assert!(reject_template("abc123", "deleted").is_ok());
    }

    #[test]
    fn schema_hash_is_deterministic() {
        let doc = OntologyDocument {
            domain: crate::builtin::poleo_ontology().domain,
            entity_types: vec![],
            relationships: vec![],
            no_self_loops: true,
        };
        let hash1 = schema_hash(&doc);
        let hash2 = schema_hash(&doc);
        assert_eq!(hash1, hash2);
        assert!(!hash1.is_empty());
    }

    #[test]
    fn require_sound_accepts_valid_document() {
        let doc = OntologyDocument {
            domain: crate::builtin::poleo_ontology().domain,
            entity_types: vec![],
            relationships: vec![],
            no_self_loops: true,
        };
        assert!(require_sound(&doc).is_ok());
    }

    #[test]
    fn require_sound_rejects_invalid_document() {
        let doc = OntologyDocument {
            domain: crate::builtin::poleo_ontology().domain,
            entity_types: vec![crate::models::EntityTypeDef {
                label: "Test".into(),
                pole_type: "INVALID".into(),
                subtype: None,
                color: None,
                icon: None,
                properties: vec![],
                description: None,
                aliases: std::collections::HashMap::new(),
                threshold: None,
                resolution_threshold: None,
                review_threshold: None,
            }],
            relationships: vec![],
            no_self_loops: true,
        };
        assert!(require_sound(&doc).is_err());
    }

    /// Ontology document with a single entity type, for mapping planning.
    fn mapping_document(label: &str, subtype: Option<&str>) -> OntologyDocument {
        OntologyDocument {
            domain: crate::models::DomainInfo {
                id: "test".to_string(),
                name: "test".to_string(),
                description: None,
                tagline: None,
                emoji: None,
            },
            entity_types: vec![crate::models::EntityTypeDef {
                label: label.to_string(),
                pole_type: "PERSON".to_string(),
                subtype: subtype.map(str::to_string),
                color: None,
                icon: None,
                properties: vec![],
                description: None,
                aliases: std::collections::HashMap::new(),
                threshold: None,
                resolution_threshold: None,
                review_threshold: None,
            }],
            relationships: vec![],
            no_self_loops: true,
        }
    }

    #[test]
    fn plan_mapping_rejects_subtype_that_is_not_a_valid_identifier() {
        let source = mapping_document("Person", None);
        let target = mapping_document("Actor", Some("Actor` DETACH DELETE n //"));
        let error = plan_mapping(&source, &target, ("Person", "Actor"))
            .err()
            .expect("an invalid subtype must be rejected");
        assert!(
            matches!(error, MemoryError::InvalidInput(_)),
            "expected invalid input, got {error:?}"
        );
    }

    #[test]
    fn plan_mapping_accepts_valid_subtype() {
        let source = mapping_document("Person", None);
        let target = mapping_document("Actor", Some("actor"));
        let step = plan_mapping(&source, &target, ("Person", "Actor")).unwrap();
        assert_eq!(step.subtype_label.as_deref(), Some("ACTOR"));
        assert_eq!(step.subtype.as_deref(), Some("ACTOR"));
    }

    #[test]
    fn ontology_node_label_validates() {
        assert!(ontology_node_label("Person").is_some());
        assert!(ontology_node_label("TvShow").is_some());
        assert!(ontology_node_label("").is_none());
        assert!(ontology_node_label("1invalid").is_none());
    }

    fn is_arrows_shape(data: &serde_json::Value) -> bool {
        if let serde_json::Value::Object(map) = data {
            map.get("nodes").is_some_and(|v| v.is_array())
                && map.get("relationships").is_some_and(|v| v.is_array())
        } else {
            false
        }
    }

    #[test]
    fn is_arrows_shape_detects_arrows() {
        let arrows = serde_json::json!({
            "nodes": [{"label": "Person"}],
            "relationships": [{"type": "WORKS_FOR"}]
        });
        assert!(is_arrows_shape(&arrows));

        let not_arrows = serde_json::json!({
            "domain": {"id": "test", "name": "Test"}
        });
        assert!(!is_arrows_shape(&not_arrows));
    }

    #[test]
    fn validate_entity_type_maps_pole_types() {
        assert_eq!(validate_entity_type("PERSON"), Some("Person".to_owned()));
        assert_eq!(
            validate_entity_type("ORGANIZATION"),
            Some("Organization".to_owned())
        );
        assert_eq!(
            validate_entity_type("LOCATION"),
            Some("Location".to_owned())
        );
        assert_eq!(validate_entity_type("EVENT"), Some("Event".to_owned()));
        assert_eq!(validate_entity_type("OBJECT"), Some("Object".to_owned()));
        assert_eq!(validate_entity_type("INVALID"), None);
    }

    #[test]
    fn bolt_param_helpers_construct_expected_types() {
        let owned = bs("hello".to_string());
        let borrowed = bs_ref("world");
        assert!(matches!(owned, BoltType::String(_)));
        assert!(matches!(borrowed, BoltType::String(_)));

        let p = params([("a", bs_ref("val1")), ("b", bs("val2".to_string()))]);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].0, "a");
        assert_eq!(p[1].0, "b");
    }

    #[test]
    fn count_entities_with_label_query_formats_cypher() {
        let q = count_entities_with_label_query("User");
        assert!(q.contains("MATCH (e:User)"));
        assert!(q.contains("RETURN count(e) AS matched"));
    }

    #[test]
    fn relabel_entities_query_generates_clauses() {
        // Simple case: no subtype, no set_type
        let q1 = relabel_entities_query("Old", "New", false, None, None);
        assert!(q1.contains("REMOVE e:`Old`"));
        assert!(q1.contains("SET e:`New`"));
        assert!(!q1.contains("subtype"));

        // With subtype and set_type
        let q2 = relabel_entities_query(
            "PersonOld",
            "PersonNew",
            true,
            Some("Person"),
            Some("ACTOR"),
        );
        assert!(q2.contains("REMOVE e:`PersonOld`"));
        assert!(q2.contains("SET e:`PersonNew`"));
        assert!(q2.contains("SET e:`ACTOR`, e.subtype = $subtype"));
        assert!(q2.contains("SET e:`Person`, e.type = $type"));
    }

    #[test]
    fn parse_inline_handles_valid_and_invalid_json() {
        // Valid JSON object
        let res = parse_inline(r#"{"nodes": [], "relationships": []}"#, "json");
        assert!(res.is_ok());

        // Malformed JSON
        let err1 = parse_inline("not json", "json");
        assert!(err1.is_err());

        // Valid JSON but array instead of object
        let err2 = parse_inline("[1, 2, 3]", "json");
        assert!(err2.is_err());
    }

    #[test]
    fn document_at_finds_revision_and_handles_errors() {
        let doc = mapping_document("Person", None);
        let ontology = Ontology {
            record: OntologyRecord {
                id: "test-ont".to_string(),
                name: "Test".to_string(),
                description: None,
                workspace_id: None,
                is_system: false,
                created_at: None,
            },
            versions: vec![
                OntologyVersion {
                    id: "test-ont:1".to_string(),
                    ontology_id: "test-ont".to_string(),
                    revision: 1,
                    validation_mode: "permissive".to_string(),
                    document: Some(doc.clone()),
                    schema_hash: None,
                    created_at: None,
                    message: None,
                },
                OntologyVersion {
                    id: "test-ont:2".to_string(),
                    ontology_id: "test-ont".to_string(),
                    revision: 2,
                    validation_mode: "strict".to_string(),
                    document: None, // unreadable
                    schema_hash: None,
                    created_at: None,
                    message: None,
                },
            ],
        };

        // Found revision 1
        let found = document_at(&ontology, 1).unwrap();
        assert_eq!(found.domain.id, doc.domain.id);

        // Unreadable revision 2
        let unreadable = document_at(&ontology, 2);
        assert!(unreadable.is_err());

        // Non-existent revision 3
        let missing = document_at(&ontology, 3);
        assert!(missing.is_err());
    }

    #[test]
    fn props_conversions_from_bolt_fixtures() {
        let mut map = neo4rs::BoltMap::new();
        map.put("id".into(), BoltType::from("v-123"));
        map.put("ontology_id".into(), BoltType::from("ont-456"));
        map.put("revision".into(), BoltType::from(2i64));
        map.put("validation_mode".into(), BoltType::from("strict"));
        map.put("name".into(), BoltType::from("Sample"));
        map.put("description".into(), BoltType::from("A sample ontology"));
        map.put("is_system".into(), BoltType::from(true));
        map.put("status".into(), BoltType::from("completed"));
        map.put("total".into(), BoltType::from(10i64));
        map.put("processed".into(), BoltType::from(10i64));
        map.put("errored".into(), BoltType::from(0i64));
        map.put("spec".into(), BoltType::from(r#"{"dry_run": false}"#));

        let doc = mapping_document("User", None);
        let doc_json = serde_json::to_string(&doc).unwrap();
        map.put("document".into(), BoltType::from(doc_json.as_str()));

        let bolt_map = BoltType::Map(map);
        let props = Props::of(&bolt_map).unwrap();

        // Test version_from_props
        let version = version_from_props(&props);
        assert_eq!(version.id, "v-123");
        assert_eq!(version.ontology_id, "ont-456");
        assert_eq!(version.revision, 2);
        assert_eq!(version.validation_mode, "strict");
        assert!(version.document.is_some());

        // Test record_from_props
        let record = record_from_props(&props);
        assert_eq!(record.id, "v-123");
        assert_eq!(record.name, "Sample");
        assert_eq!(record.description.as_deref(), Some("A sample ontology"));
        assert!(record.is_system);

        // Test migration_from_props
        let migration = migration_from_props(&props);
        assert_eq!(migration.id, "v-123");
        assert_eq!(migration.ontology_id.as_deref(), Some("ont-456"));
        assert_eq!(migration.status.as_deref(), Some("completed"));
        assert_eq!(migration.total, Some(10));
        assert_eq!(migration.processed, Some(10));
        assert_eq!(migration.errored, Some(0));
        assert!(migration.spec.is_some());
    }

    #[test]
    fn compute_diff_calculates_added_removed_entities_and_relations() {
        let mut doc1 = mapping_document("Person", None);
        doc1.relationships.push(crate::models::RelationshipDef {
            source: "Person".to_string(),
            r#type: "KNOWS".to_string(),
            target: "Person".to_string(),
            description: None,
            threshold: None,
            inverse: None,
            unique_source: false,
            unique_target: false,
            acyclic: false,
            allow_self: false,
        });

        let mut doc2 = mapping_document("Actor", Some("actor"));
        doc2.relationships.push(crate::models::RelationshipDef {
            source: "Actor".to_string(),
            r#type: "ACTS_IN".to_string(),
            target: "Actor".to_string(),
            description: None,
            threshold: None,
            inverse: None,
            unique_source: false,
            unique_target: false,
            acyclic: false,
            allow_self: false,
        });

        let ontology = Ontology {
            record: OntologyRecord {
                id: "ont-diff".to_string(),
                name: "Diff Test".to_string(),
                description: None,
                workspace_id: None,
                is_system: false,
                created_at: None,
            },
            versions: vec![
                OntologyVersion {
                    id: "ont-diff:1".to_string(),
                    ontology_id: "ont-diff".to_string(),
                    revision: 1,
                    validation_mode: "permissive".to_string(),
                    document: Some(doc1),
                    schema_hash: None,
                    created_at: None,
                    message: None,
                },
                OntologyVersion {
                    id: "ont-diff:2".to_string(),
                    ontology_id: "ont-diff".to_string(),
                    revision: 2,
                    validation_mode: "strict".to_string(),
                    document: Some(doc2),
                    schema_hash: None,
                    created_at: None,
                    message: None,
                },
            ],
        };

        let diff = compute_diff(&ontology, 1, 2).expect("diff should succeed");
        assert_eq!(diff.from_revision, Some(1));
        assert_eq!(diff.to_revision, Some(2));

        // Validation mode changed
        assert!(diff.mode_change.is_some());
        let mode_change = diff.mode_change.unwrap();
        assert_eq!(mode_change["from"], "permissive");
        assert_eq!(mode_change["to"], "strict");

        // Entities added & removed
        let entities = diff.entity_types.as_object().unwrap();
        assert!(entities.contains_key("added"));
        assert!(entities.contains_key("removed"));

        // Relations added & removed
        let rels = diff.relationships.as_object().unwrap();
        assert!(rels.contains_key("added"));
        assert!(rels.contains_key("removed"));
    }

    #[test]
    fn template_summary_reports_builtin_template_metadata() {
        let summary = template_summary("poleo");

        assert_eq!(summary.id, "template:poleo");
        assert_eq!(summary.name, "poleo");
        assert_eq!(summary.display_name.as_deref(), Some("poleo"));
        assert_eq!(
            summary.description.as_deref(),
            Some(
                "Person, Object, Location, Event, Organization — the default investigative entity model."
            )
        );
        assert_eq!(
            summary.tagline.as_deref(),
            Some("The default POLE+O entity model")
        );
        assert_eq!(summary.emoji, None);
        assert!(summary.is_system);
        assert_eq!(summary.current_revision, None);
        assert!(!summary.is_active);
    }

    #[test]
    fn template_ontology_builds_system_record_with_one_initial_version() {
        let ontology = template_ontology("template:poleo").expect("built-in template resolves");

        assert_eq!(ontology.record.id, "template:poleo");
        assert_eq!(ontology.record.name, "poleo");
        assert_eq!(
            ontology.record.description.as_deref(),
            Some(
                "Person, Object, Location, Event, Organization — the default investigative entity model."
            )
        );
        assert_eq!(ontology.record.workspace_id, None);
        assert!(ontology.record.is_system);
        assert_eq!(ontology.record.created_at, None);

        assert_eq!(ontology.versions.len(), 1);
        let version = &ontology.versions[0];
        assert_eq!(version.id, "template:poleo:1");
        assert_eq!(version.ontology_id, "template:poleo");
        assert_eq!(version.revision, 1);
        assert_eq!(version.validation_mode, "permissive");
        assert_eq!(version.schema_hash, None);
        assert_eq!(version.created_at, None);
        assert_eq!(version.message, None);

        let document = version
            .document
            .as_ref()
            .expect("template version carries its document");
        let labels = document.labels();
        assert_eq!(labels.len(), 5);
        for label in ["Person", "Organization", "Location", "Event", "Object"] {
            assert!(labels.contains(&label), "missing label {label}");
        }
    }

    #[test]
    fn template_ontology_accepts_a_bare_template_name() {
        let ontology = template_ontology("poleo").expect("built-in template resolves");

        assert_eq!(ontology.record.id, "poleo");
        assert_eq!(ontology.record.name, "poleo");
        assert_eq!(ontology.versions.len(), 1);
        assert_eq!(ontology.versions[0].id, "poleo:1");
        assert_eq!(ontology.versions[0].ontology_id, "poleo");
    }

    #[test]
    fn template_ontology_panics_for_an_unknown_template() {
        // Current behaviour: `get_template` panics on unknown names, so an
        // unknown template id aborts instead of returning an error.
        let outcome = std::panic::catch_unwind(|| template_ontology("template:unknown"));

        let payload = outcome.expect_err("unknown template ids must not resolve");
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or_default();
        assert!(
            message.contains("Unknown ontology template"),
            "unexpected panic message: {message}"
        );
    }
}

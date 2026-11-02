//! Long-term memory bridge methods (Silver entities/preferences/facts, Gold
//! relationships and merging, Platinum entity operations).

use muninn_application::{
    AddEntityOptions, AddPreferenceOptions, AddRelationshipOptions, SearchEntitiesOptions,
    SearchPreferencesOptions,
};
use muninn_core::MemoryError;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::wire::{self, Args};
use crate::{BridgeError, BridgeResult, BridgeState, Reply};

/// Default page size for `list_entities` when the caller sends no limit.
const DEFAULT_LIST_LIMIT: i64 = 100;

pub async fn dispatch(state: &BridgeState, method: &str, args: &Args) -> Option<BridgeResult> {
    let result = match method {
        "add_entity" => add_entity(state, args).await,
        "add_preference" => add_preference(state, args).await,
        "add_fact" => add_fact(state, args).await,
        "search_entities" => search_entities(state, args).await,
        "search_preferences" => search_preferences(state, args).await,
        "get_entity_by_name" => get_entity_by_name(state, args).await,
        "get_related_entities" => get_related_entities(state, args).await,
        "add_relationship" => add_relationship(state, args).await,
        "merge_duplicate_entities" => merge_duplicate_entities(state, args).await,
        "list_entities" => list_entities(state, args).await,
        "get_entity" => get_entity(state, args).await,
        "update_entity" => update_entity(state, args).await,
        "delete_entity" => delete_entity(state, args).await,
        "set_entity_feedback" => set_entity_feedback(state, args).await,
        "get_entity_history" => get_entity_history(state, args).await,
        "merge_entities" => merge_entities(state, args).await,
        "get_entity_graph" => get_entity_graph(state, args).await,
        _ => return None,
    };
    Some(result)
}

fn entity_not_found(id: Uuid) -> BridgeError {
    BridgeError::Memory(MemoryError::not_found(format!("entity {id}")))
}

fn entities(items: &[muninn_core::model::Entity]) -> Reply {
    Reply::Json(Value::Array(items.iter().map(wire::entity).collect()))
}

/// The TCK reference adapter maps this onto
/// `add_entity(resolve=False, deduplicate=False, geocode=False, enrich=False)`.
async fn add_entity(state: &BridgeState, args: &Args) -> BridgeResult {
    let options = AddEntityOptions {
        description: args.opt_string("description")?,
        ..AddEntityOptions::default()
    };
    let entity = state
        .memory
        .long_term()
        .add_entity(&args.string("name")?, &args.string("entity_type")?, options)
        .await?;
    Ok(Reply::Json(wire::entity(&entity)))
}

async fn add_preference(state: &BridgeState, args: &Args) -> BridgeResult {
    let options = AddPreferenceOptions {
        user_identifier: args.opt_string("user_identifier")?,
        context: args.opt_string("context")?,
        ..AddPreferenceOptions::default()
    };
    let preference = state
        .memory
        .long_term()
        .add_preference(
            &args.string("category")?,
            &args.string("preference")?,
            options,
        )
        .await?;
    Ok(Reply::Json(wire::preference(&preference)))
}

/// The TCK sends the object as `obj` (Python keyword clash).
async fn add_fact(state: &BridgeState, args: &Args) -> BridgeResult {
    let fact = state
        .memory
        .long_term()
        .add_fact(
            &args.string("subject")?,
            &args.string("predicate")?,
            &args.string("obj")?,
            muninn_application::AddFactOptions::default(),
        )
        .await?;
    Ok(Reply::Json(wire::fact(&fact)))
}

/// The TCK reference adapter searches with `threshold=0.0`: every indexed
/// entity is a candidate, ranked by similarity.
async fn search_entities(state: &BridgeState, args: &Args) -> BridgeResult {
    let options = SearchEntitiesOptions {
        limit: args.opt_i64("limit")?.unwrap_or(10),
        threshold: 0.0,
        ..SearchEntitiesOptions::default()
    };
    let found = state
        .memory
        .long_term()
        .search_entities(&args.string("query")?, &options)
        .await?;
    Ok(entities(&found))
}

async fn search_preferences(state: &BridgeState, args: &Args) -> BridgeResult {
    let options = SearchPreferencesOptions {
        user_identifier: args.opt_string("user_identifier")?,
        category: args.opt_string("category")?,
        limit: args.opt_i64("limit")?.unwrap_or(10),
        ..SearchPreferencesOptions::default()
    };
    let found = state
        .memory
        .long_term()
        .search_preferences(&args.string("query")?, &options)
        .await?;
    Ok(Reply::Json(Value::Array(
        found.iter().map(wire::preference).collect(),
    )))
}

/// Absent entities are reported as JSON `null`, which the TCK maps to `None`.
async fn get_entity_by_name(state: &BridgeState, args: &Args) -> BridgeResult {
    let entity = state
        .memory
        .long_term()
        .get_entity_by_name(&args.string("name")?)
        .await?;
    Ok(Reply::Json(
        entity.as_ref().map_or(Value::Null, wire::entity),
    ))
}

async fn get_related_entities(state: &BridgeState, args: &Args) -> BridgeResult {
    let types: Vec<String> = args.opt_string("relationship_type")?.into_iter().collect();
    let related = state
        .memory
        .long_term()
        .get_related_entities(
            args.uuid("entity_id")?,
            &types,
            args.opt_i64("depth")?.unwrap_or(1),
        )
        .await?;
    Ok(Reply::Json(Value::Array(
        related
            .iter()
            .map(|item| wire::entity(&item.entity))
            .collect(),
    )))
}

async fn add_relationship(state: &BridgeState, args: &Args) -> BridgeResult {
    let options = AddRelationshipOptions {
        properties: args.object("properties")?,
        ..AddRelationshipOptions::default()
    };
    let relationship = state
        .memory
        .long_term()
        .add_relationship(
            args.uuid("source_id")?,
            args.uuid("target_id")?,
            &args.string("relationship_type")?,
            options,
        )
        .await?;
    Ok(Reply::Json(wire::relationship(&relationship)))
}

/// Returns the surviving (target) entity, as the TCK reference adapter does.
async fn merge_duplicate_entities(state: &BridgeState, args: &Args) -> BridgeResult {
    let source_id = args.uuid("source_id")?;
    let target_id = args.uuid("target_id")?;
    let canonical_name = args.opt_string("canonical_name")?;
    let (_, target) = state
        .memory
        .long_term()
        .merge_duplicate_entities(source_id, target_id, canonical_name.as_deref())
        .await?
        .ok_or_else(|| {
            BridgeError::Memory(MemoryError::not_found(format!(
                "merge failed: entities {source_id} or {target_id} not found"
            )))
        })?;
    Ok(Reply::Json(wire::entity(&target)))
}

async fn list_entities(state: &BridgeState, args: &Args) -> BridgeResult {
    let found = state
        .memory
        .long_term()
        .list_entities(
            args.opt_string("type")?.as_deref(),
            args.opt_i64("limit")?.unwrap_or(DEFAULT_LIST_LIMIT),
        )
        .await?;
    Ok(entities(&found))
}

/// One entity with its relationships (hosted `GET /entities/{id}`).
async fn get_entity(state: &BridgeState, args: &Args) -> BridgeResult {
    let id = args.uuid("entity_id")?;
    let memory = state.memory.long_term();
    let entity = memory
        .get_entity(id)
        .await?
        .ok_or_else(|| entity_not_found(id))?;
    let related = memory.get_related_entities(id, &[], 1).await?;
    let mut value = wire::entity(&entity);
    value["relationships"] = Value::Array(
        related
            .iter()
            .map(|item| wire::relationship(&item.relationship))
            .collect(),
    );
    Ok(Reply::Json(value))
}

async fn update_entity(state: &BridgeState, args: &Args) -> BridgeResult {
    let id = args.uuid("entity_id")?;
    let name = args.opt_string("name")?;
    let description = args.opt_string("description")?;
    let entity = state
        .memory
        .long_term()
        .update_entity(id, name.as_deref(), description.as_deref())
        .await?
        .ok_or_else(|| entity_not_found(id))?;
    Ok(Reply::Json(wire::entity(&entity)))
}

/// Idempotent, like `delete_conversation`.
async fn delete_entity(state: &BridgeState, args: &Args) -> BridgeResult {
    state
        .memory
        .long_term()
        .delete_entity(args.uuid("entity_id")?)
        .await?;
    Ok(Reply::NoContent)
}

async fn set_entity_feedback(state: &BridgeState, args: &Args) -> BridgeResult {
    let id = args.uuid("entity_id")?;
    let user_score = args
        .opt_f64("user_score")?
        .ok_or_else(|| BridgeError::BadRequest("missing argument 'user_score'".into()))?;
    let confirmed = args.opt_bool("confirmed")?.unwrap_or(false);
    let updated = state
        .memory
        .long_term()
        .set_entity_feedback(id, user_score, confirmed)
        .await?;
    Ok(Reply::Json(json!({ "id": id, "updated": updated })))
}

async fn get_entity_history(state: &BridgeState, args: &Args) -> BridgeResult {
    let id = args.uuid("entity_id")?;
    let mentions = state
        .memory
        .long_term()
        .get_entity_history(id)
        .await?
        .ok_or_else(|| entity_not_found(id))?;
    Ok(Reply::Json(wire::entity_history(id, &mentions)))
}

async fn merge_entities(state: &BridgeState, args: &Args) -> BridgeResult {
    let source_id = args.uuid("source_id")?;
    let target_id = args.uuid("target_id")?;
    state
        .memory
        .long_term()
        .merge_entities(source_id, target_id)
        .await?
        .ok_or_else(|| {
            BridgeError::Memory(MemoryError::not_found(format!(
                "merge failed: entities {source_id} or {target_id} not found"
            )))
        })?;
    Ok(Reply::Json(json!({
        "source_id": source_id,
        "target_id": target_id,
        "status": "merged",
    })))
}

async fn get_entity_graph(state: &BridgeState, args: &Args) -> BridgeResult {
    let limit = args
        .opt_i64("limit")?
        .unwrap_or(muninn_application::long_term::DEFAULT_GRAPH_LIMIT);
    let graph = state.memory.long_term().get_entity_graph(limit).await?;
    Ok(Reply::Json(wire::entity_graph(&graph)))
}

//! MCP resource definitions for Muninn.
//!
//! Equivalent to `neo4j_agent_memory.mcp._resources`.
//!
//! Resources provide auto-injected context to the LLM. They are organized
//! into profiles:
//! - **Core**: `memory://context/{session_id}` (assembled context for a session)
//! - **Extended**: `memory://entities`, `memory://preferences`, `memory://graph/stats`

use serde_json::{json, Value};

/// A resource URI template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceUri {
    /// The full URI template (e.g., "memory://context/{session_id}").
    pub template: String,
    /// Description of what this resource provides.
    pub description: String,
    /// MIME type of the resource content.
    pub mime_type: String,
}

/// Profile determines which resources are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResourceProfile {
    /// Only core resources (context template).
    Core,
    /// All resources including extended (default).
    #[default]
    Extended,
}

/// Returns the core resource: `memory://context/{session_id}`.
///
/// Assembled context for a session — conversation history, relevant entities,
/// preferences, and similar reasoning traces. Suitable for embedding in the
/// system prompt at conversation start.
pub fn context_resource() -> ResourceUri {
    ResourceUri {
        template: "memory://context/{session_id}".to_owned(),
        description: "Assembled context for a session.".to_owned(),
        mime_type: "application/json".to_owned(),
    }
}

/// Returns the extended resource: `memory://entities`.
///
/// Catalog of all entities in the knowledge graph — names, types, descriptions,
/// and relationship counts.
pub fn entities_resource() -> ResourceUri {
    ResourceUri {
        template: "memory://entities".to_owned(),
        description: "Catalog of all entities in the knowledge graph.".to_owned(),
        mime_type: "application/json".to_owned(),
    }
}

/// Returns the extended resource: `memory://preferences`.
///
/// All stored user preferences organized by category.
pub fn preferences_resource() -> ResourceUri {
    ResourceUri {
        template: "memory://preferences".to_owned(),
        description: "All stored user preferences organized by category.".to_owned(),
        mime_type: "application/json".to_owned(),
    }
}

/// Returns the extended resource: `memory://graph/stats`.
///
/// Knowledge graph statistics — node/relationship counts and entity type distribution.
pub fn graph_stats_resource() -> ResourceUri {
    ResourceUri {
        template: "memory://graph/stats".to_owned(),
        description: "Knowledge graph statistics.".to_owned(),
        mime_type: "application/json".to_owned(),
    }
}

/// Returns all resources available for the given profile.
pub fn list_resources(profile: ResourceProfile) -> Vec<ResourceUri> {
    let mut resources = vec![context_resource()];

    if profile == ResourceProfile::Extended {
        resources.push(entities_resource());
        resources.push(preferences_resource());
        resources.push(graph_stats_resource());
    }

    resources
}

/// Returns a resource by URI template, if it exists in the given profile.
pub fn get_resource(uri: &str, profile: ResourceProfile) -> Option<ResourceUri> {
    match uri {
        "memory://context/{session_id}" => Some(context_resource()),
        "memory://entities" if profile == ResourceProfile::Extended => Some(entities_resource()),
        "memory://preferences" if profile == ResourceProfile::Extended => {
            Some(preferences_resource())
        }
        "memory://graph/stats" if profile == ResourceProfile::Extended => {
            Some(graph_stats_resource())
        }
        _ => None,
    }
}

/// Build the JSON content for the context resource.
///
/// This is a helper that assembles the context payload when a client reads
/// `memory://context/{session_id}`. The actual data comes from the memory
/// client; this function structures it for the MCP response.
pub fn build_context_content(session_id: &str, context: &Value) -> Value {
    json!({
        "session_id": session_id,
        "context": context,
        "has_context": context != &Value::Null && context != &json!({}),
    })
}

/// Build the JSON content for the entities resource.
///
/// Takes a list of entity objects and returns the structured response.
pub fn build_entities_content(entities: &[Value]) -> Value {
    json!({
        "entity_count": entities.len(),
        "entities": entities,
    })
}

/// Build the JSON content for the preferences resource.
pub fn build_preferences_content(preferences: &[Value]) -> Value {
    json!({
        "preference_count": preferences.len(),
        "preferences": preferences,
    })
}

/// Build the JSON content for the graph stats resource.
pub fn build_graph_stats_content(stats: &Value) -> Value {
    json!({
        "stats": stats,
    })
}

/// Build an error response for a resource read failure.
pub fn build_error_content(error: &str) -> Value {
    json!({
        "error": error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_resource_has_correct_uri() {
        let resource = context_resource();
        assert_eq!(resource.template, "memory://context/{session_id}");
        assert_eq!(resource.mime_type, "application/json");
    }

    #[test]
    fn entities_resource_has_correct_uri() {
        let resource = entities_resource();
        assert_eq!(resource.template, "memory://entities");
    }

    #[test]
    fn preferences_resource_has_correct_uri() {
        let resource = preferences_resource();
        assert_eq!(resource.template, "memory://preferences");
    }

    #[test]
    fn graph_stats_resource_has_correct_uri() {
        let resource = graph_stats_resource();
        assert_eq!(resource.template, "memory://graph/stats");
    }

    #[test]
    fn list_resources_core_returns_one() {
        let resources = list_resources(ResourceProfile::Core);
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].template, "memory://context/{session_id}");
    }

    #[test]
    fn list_resources_extended_returns_four() {
        let resources = list_resources(ResourceProfile::Extended);
        assert_eq!(resources.len(), 4);
        assert_eq!(resources[0].template, "memory://context/{session_id}");
        assert_eq!(resources[1].template, "memory://entities");
        assert_eq!(resources[2].template, "memory://preferences");
        assert_eq!(resources[3].template, "memory://graph/stats");
    }

    #[test]
    fn get_resource_finds_core_resource() {
        let resource = get_resource("memory://context/{session_id}", ResourceProfile::Core);
        assert!(resource.is_some());
    }

    #[test]
    fn get_resource_finds_extended_resource() {
        let resource = get_resource("memory://entities", ResourceProfile::Extended);
        assert!(resource.is_some());
    }

    #[test]
    fn get_resource_returns_none_for_core_extended() {
        let resource = get_resource("memory://entities", ResourceProfile::Core);
        assert!(resource.is_none());
    }

    #[test]
    fn get_resource_returns_none_for_unknown() {
        let resource = get_resource("memory://unknown", ResourceProfile::Extended);
        assert!(resource.is_none());
    }

    #[test]
    fn build_context_content_includes_session_id() {
        let content = build_context_content("test-session", &json!({"messages": []}));
        assert_eq!(content["session_id"], "test-session");
        assert_eq!(content["has_context"], true);
    }

    #[test]
    fn build_context_content_empty_is_false() {
        let content = build_context_content("test-session", &json!({}));
        assert_eq!(content["has_context"], false);
    }

    #[test]
    fn build_entities_content_counts_entities() {
        let entities = vec![
            json!({"id": "1", "name": "Alice"}),
            json!({"id": "2", "name": "Bob"}),
        ];
        let content = build_entities_content(&entities);
        assert_eq!(content["entity_count"], 2);
        assert_eq!(content["entities"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn build_preferences_content_counts_preferences() {
        let prefs = vec![json!({"category": "food", "preference": "Italian"})];
        let content = build_preferences_content(&prefs);
        assert_eq!(content["preference_count"], 1);
    }

    #[test]
    fn build_graph_stats_content_wraps_stats() {
        let stats = json!({"nodes": 100, "relationships": 50});
        let content = build_graph_stats_content(&stats);
        assert_eq!(content["stats"]["nodes"], 100);
    }

    #[test]
    fn build_error_content_includes_error() {
        let content = build_error_content("Database connection failed");
        assert_eq!(content["error"], "Database connection failed");
    }

    #[test]
    fn resource_profile_default_is_extended() {
        let profile = ResourceProfile::default();
        assert_eq!(profile, ResourceProfile::Extended);
    }
}

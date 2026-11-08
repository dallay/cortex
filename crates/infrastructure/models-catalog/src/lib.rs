use std::collections::HashMap;

use async_trait::async_trait;
use rook_core::{
    ConnectionId, ModelCapabilities, ModelCatalogEntry, ModelCatalogPort, ModelDeclaration,
};

/// Adds a model to the catalog unless the provider/model pair already exists.
fn push_unique_model_entry(
    entries: &mut Vec<ModelCatalogEntry>,
    provider_id: &str,
    model_id: &str,
) {
    if !entries
        .iter()
        .any(|entry| entry.provider_id == provider_id && entry.model_id == model_id)
    {
        entries.push(ModelCatalogEntry {
            model_id: model_id.to_string(),
            provider_id: provider_id.to_string(),
        });
    }
}

/// Immutable declarative catalog. Runtime discovery can supply IDs to `resolve`
/// without making unverified capability assertions.
pub struct StaticModelCatalog {
    /// Pre-merged global catalog entries for `list()`.
    global_entries: Vec<ModelCatalogEntry>,
    /// Pre-merged global capabilities keyed by `(provider_id, model_id)`.
    global_capabilities: HashMap<(String, String), ModelCapabilities>,
    /// Pre-merged connection-scoped capabilities keyed by `(provider_id, connection_id, model_id)`.
    connection_capabilities: HashMap<(String, ConnectionId, String), ModelCapabilities>,
    /// Pre-merged connection-scoped additional entries for `list_for_connection()`.
    connection_entries: HashMap<(String, ConnectionId), Vec<ModelCatalogEntry>>,
}

impl Default for StaticModelCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl StaticModelCatalog {
    pub fn new() -> Self {
        Self::with_declarations(Vec::new()).expect("bundled catalog is valid")
    }

    pub fn with_declarations(mut configured: Vec<ModelDeclaration>) -> Result<Self, String> {
        let mut bundled: Vec<ModelDeclaration> = serde_json::from_str(include_str!("models.json"))
            .map_err(|e| format!("invalid bundled catalog: {e}"))?;
        for entry in bundled.iter_mut().chain(configured.iter_mut()) {
            entry.provider_id = rook_core::descriptor_for(&entry.provider_id)
                .ok_or_else(|| format!("unknown provider id: {}", entry.provider_id))?
                .id
                .to_string();
        }
        for (i, entry) in configured.iter().enumerate() {
            if entry.model_id.trim().is_empty() || entry.model_id.trim() != entry.model_id {
                return Err(
                    "catalog model_id must be nonempty without surrounding whitespace".into(),
                );
            }
            if entry.capabilities.context_tokens == Some(0) {
                return Err("context_tokens must be positive".into());
            }
            if configured[..i].iter().any(|other| {
                other.provider_id == entry.provider_id
                    && other.connection_id == entry.connection_id
                    && other.model_id == entry.model_id
            }) {
                return Err("duplicate catalog declaration".into());
            }
        }

        // Build merged global catalog view
        let mut global_entries = Vec::new();
        let mut global_capabilities: HashMap<(String, String), ModelCapabilities> = HashMap::new();

        // Level 1: Bundled declarations
        for entry in &bundled {
            let key = (entry.provider_id.clone(), entry.model_id.clone());
            let caps = global_capabilities.entry(key.clone()).or_default();
            caps.apply(&entry.capabilities, "bundled");
            push_unique_model_entry(&mut global_entries, &entry.provider_id, &entry.model_id);
        }

        // Level 2: Configured global declarations (connection_id.is_none())
        for entry in configured.iter().filter(|e| e.connection_id.is_none()) {
            let key = (entry.provider_id.clone(), entry.model_id.clone());
            let caps = global_capabilities.entry(key.clone()).or_default();
            caps.apply(&entry.capabilities, "provider_configuration");
            push_unique_model_entry(&mut global_entries, &entry.provider_id, &entry.model_id);
        }

        // Level 3: Descriptors with inline static model lists contribute to the catalog
        for descriptor in rook_core::PROVIDER_DESCRIPTORS {
            if let rook_core::ModelSource::Static(models) = descriptor.models {
                for model_id in models {
                    let key = (descriptor.id.to_string(), (*model_id).to_string());
                    global_capabilities.entry(key).or_default();
                    push_unique_model_entry(&mut global_entries, descriptor.id.as_ref(), model_id);
                }
            }
        }

        // Level 4: Connection-scoped declarations
        let mut connection_capabilities: HashMap<
            (String, ConnectionId, String),
            ModelCapabilities,
        > = HashMap::new();
        let mut connection_entries: HashMap<(String, ConnectionId), Vec<ModelCatalogEntry>> =
            HashMap::new();

        for entry in configured.iter().filter(|e| e.connection_id.is_some()) {
            let conn_id = entry.connection_id.as_ref().unwrap();
            let key = (entry.provider_id.clone(), *conn_id, entry.model_id.clone());

            // Precedence: start from global capabilities for (provider_id, model_id), then apply connection overrides
            let base_caps = global_capabilities
                .get(&(entry.provider_id.clone(), entry.model_id.clone()))
                .cloned()
                .unwrap_or_default();
            let mut caps = base_caps;
            caps.apply(&entry.capabilities, "connection_configuration");
            connection_capabilities.insert(key, caps);

            // Record in connection_entries if not already in global_entries
            if !global_entries.iter().any(|e: &ModelCatalogEntry| {
                e.provider_id == entry.provider_id && e.model_id == entry.model_id
            }) {
                let conn_key = (entry.provider_id.clone(), *conn_id);
                let list = connection_entries.entry(conn_key).or_default();
                if !list.iter().any(|e| e.model_id == entry.model_id) {
                    list.push(ModelCatalogEntry {
                        model_id: entry.model_id.clone(),
                        provider_id: entry.provider_id.clone(),
                    });
                }
            }
        }

        Ok(Self {
            global_entries,
            global_capabilities,
            connection_capabilities,
            connection_entries,
        })
    }
}

#[async_trait]
impl ModelCatalogPort for StaticModelCatalog {
    async fn list(&self) -> Vec<ModelCatalogEntry> {
        self.global_entries.clone()
    }

    async fn resolve(
        &self,
        provider: &'static rook_core::ProviderDescriptor,
        connection: Option<&ConnectionId>,
        model: &str,
    ) -> ModelCapabilities {
        if let Some(conn) = connection {
            let conn_key = (provider.id.to_string(), *conn, model.to_string());
            if let Some(caps) = self.connection_capabilities.get(&conn_key) {
                return caps.clone();
            }
        }

        let global_key = (provider.id.to_string(), model.to_string());
        self.global_capabilities
            .get(&global_key)
            .cloned()
            .unwrap_or_default()
    }

    async fn list_for_connection(
        &self,
        provider: &'static rook_core::ProviderDescriptor,
        connection: &ConnectionId,
    ) -> Vec<ModelCatalogEntry> {
        let mut out = self.global_entries.clone();
        let conn_key = (provider.id.to_string(), *connection);
        if let Some(extras) = self.connection_entries.get(&conn_key) {
            out.extend(extras.iter().cloned());
        }
        out
    }
}

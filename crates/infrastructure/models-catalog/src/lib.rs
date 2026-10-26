use async_trait::async_trait;
use rook_core::{
    ConnectionId, ModelCapabilities, ModelCatalogEntry, ModelCatalogPort, ModelDeclaration,
    ProviderKind,
};

/// Immutable declarative catalog. Runtime discovery can supply IDs to `resolve`
/// without making unverified capability assertions.
pub struct StaticModelCatalog {
    bundled: Vec<ModelDeclaration>,
    configured: Vec<ModelDeclaration>,
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

    pub fn with_declarations(configured: Vec<ModelDeclaration>) -> Result<Self, String> {
        let bundled = serde_json::from_str(include_str!("models.json"))
            .map_err(|e| format!("invalid bundled catalog: {e}"))?;
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
                other.provider_kind == entry.provider_kind
                    && other.connection_id == entry.connection_id
                    && other.model_id == entry.model_id
            }) {
                return Err("duplicate catalog declaration".into());
            }
        }
        Ok(Self {
            bundled,
            configured,
        })
    }
}

#[async_trait]
impl ModelCatalogPort for StaticModelCatalog {
    async fn list(&self) -> Vec<ModelCatalogEntry> {
        let mut out: Vec<ModelCatalogEntry> = Vec::new();
        for entry in self
            .bundled
            .iter()
            .chain(&self.configured)
            .filter(|e| e.connection_id.is_none())
        {
            if !out
                .iter()
                .any(|e| e.provider_kind == entry.provider_kind && e.model_id == entry.model_id)
            {
                out.push(ModelCatalogEntry {
                    model_id: entry.model_id.clone(),
                    provider_kind: entry.provider_kind,
                });
            }
        }
        out
    }

    async fn resolve(
        &self,
        kind: ProviderKind,
        connection: Option<&ConnectionId>,
        model: &str,
    ) -> ModelCapabilities {
        let mut out = ModelCapabilities::default();
        for (entries, scoped, source) in [
            (&self.bundled, false, "bundled"),
            (&self.configured, false, "provider_configuration"),
            (&self.configured, true, "connection_configuration"),
        ] {
            for entry in entries
                .iter()
                .filter(|e| e.provider_kind == kind && e.model_id == model)
            {
                let applies = if scoped {
                    connection.is_some() && entry.connection_id.as_ref() == connection
                } else {
                    entry.connection_id.is_none()
                };
                if applies {
                    out.apply(&entry.capabilities, source);
                }
            }
        }
        out
    }

    async fn list_for_connection(
        &self,
        kind: ProviderKind,
        connection: &ConnectionId,
    ) -> Vec<ModelCatalogEntry> {
        let mut out = self.list().await;
        for entry in self
            .configured
            .iter()
            .filter(|e| e.provider_kind == kind && e.connection_id.as_ref() == Some(connection))
        {
            if !out
                .iter()
                .any(|e| e.provider_kind == kind && e.model_id == entry.model_id)
            {
                out.push(ModelCatalogEntry {
                    model_id: entry.model_id.clone(),
                    provider_kind: kind,
                });
            }
        }
        out
    }
}

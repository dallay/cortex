use async_trait::async_trait;
use rook_core::{
    ConnectionId, ModelCapabilities, ModelCatalogEntry, ModelCatalogPort, ModelDeclaration,
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
        Ok(Self {
            bundled,
            configured,
        })
    }
}

/// Resolve canonical or legacy provider ids to the canonical descriptor id
/// before duplicate validation and storage, so a legacy spelling never
/// silently misses its provider's listings and capabilities.
fn canonicalize(mut declarations: Vec<ModelDeclaration>) -> Result<Vec<ModelDeclaration>, String> {
    for declaration in &mut declarations {
        let descriptor = rook_core::descriptor_for(&declaration.provider_id)
            .ok_or_else(|| format!("unknown provider id: {}", declaration.provider_id))?;
        declaration.provider_id = descriptor.id.to_string();
    }
    Ok(declarations)
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
                .any(|e| e.provider_id == entry.provider_id && e.model_id == entry.model_id)
            {
                out.push(ModelCatalogEntry {
                    model_id: entry.model_id.clone(),
                    provider_id: entry.provider_id.clone(),
                });
            }
        }
        // Descriptors with inline model lists contribute to the catalog so
        // every listing site derives from the same table.
        for descriptor in rook_core::PROVIDER_DESCRIPTORS {
            if let rook_core::ModelSource::Static(models) = descriptor.models {
                for model_id in models {
                    if !out
                        .iter()
                        .any(|e| e.provider_id == descriptor.id && e.model_id == *model_id)
                    {
                        out.push(ModelCatalogEntry {
                            model_id: (*model_id).to_string(),
                            provider_id: descriptor.id.to_string(),
                        });
                    }
                }
            }
        }
        out
    }

    async fn resolve(
        &self,
        provider: &'static rook_core::ProviderDescriptor,
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
                .filter(|e| e.provider_id == provider.id && e.model_id == model)
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
        provider: &'static rook_core::ProviderDescriptor,
        connection: &ConnectionId,
    ) -> Vec<ModelCatalogEntry> {
        let mut out = self.list().await;
        for entry in self.configured.iter().filter(|e| {
            e.provider_id == provider.id && e.connection_id.as_ref() == Some(connection)
        }) {
            if !out
                .iter()
                .any(|e| e.provider_id == provider.id && e.model_id == entry.model_id)
            {
                out.push(ModelCatalogEntry {
                    model_id: entry.model_id.clone(),
                    provider_id: provider.id.to_string(),
                });
            }
        }
        out
    }
}

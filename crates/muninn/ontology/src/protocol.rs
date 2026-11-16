//! The ``client.ontology`` surface, as a backend-neutral trait.
//!
//! Both implementations satisfy it: [`BoltOntology`] (bolt, local Neo4j) and
//! any future hosted backend (NAMS, REST). Typing the accessor as
//! [`OntologyApi`] keeps call sites backend-agnostic; operations a backend
//! cannot honour return [`MemoryError::NotSupported`] at runtime.
//!
//! This module provides the Rust equivalent of `neo4j_agent_memory.ontology.protocol`.

use async_trait::async_trait;
use muninn_core::MemoryResult;

use crate::models::{
    ActiveOntology, MigrationJob, Ontology, OntologyDiff, OntologyDocument, OntologyImportResult,
    OntologySummary, OntologyVersion,
};

/// The ontology lifecycle: templates, revisions, activation, migration.
///
/// Mirrors `neo4j_agent_memory.ontology.protocol.OntologyAPI`.
///
/// Implementors store ontologies in different backends (bolt, REST, etc.)
/// but expose the same lifecycle surface. Operations a backend cannot honour
/// return [`MemoryError::NotSupported`].
#[async_trait]
pub trait OntologyApi: Send + Sync {
    /// List system templates + workspace-owned ontologies (active flagged).
    async fn list(&self) -> MemoryResult<Vec<OntologySummary>>;

    /// Return one ontology with its full revision history.
    async fn get(&self, ontology_id: &str) -> MemoryResult<Ontology>;

    /// Return the active ontology's parsed body + composed version metadata.
    async fn get_active(&self) -> MemoryResult<ActiveOntology>;

    /// Clone a system template into an editable copy (revision 1).
    async fn clone(&self, template_name: &str) -> MemoryResult<OntologyVersion>;

    /// Create a new ontology from a schema body (revision 1).
    async fn create(
        &self,
        name: &str,
        document: &OntologyDocument,
        validation_mode: Option<&str>,
    ) -> MemoryResult<OntologyVersion>;

    /// Create a new immutable revision (n+1) from an updated schema body.
    async fn update(
        &self,
        ontology_id: &str,
        document: &OntologyDocument,
        validation_mode: Option<&str>,
    ) -> MemoryResult<OntologyVersion>;

    /// Bind a version; subsequent entity writes validate against it.
    async fn activate(&self, version_id: &str) -> MemoryResult<OntologyVersion>;

    /// Delete an ontology and its revisions.
    async fn delete(&self, ontology_id: &str) -> MemoryResult<()>;

    /// Convert an external graph/ontology document into a native draft.
    async fn import_(
        &self,
        content: Option<&str>,
        url: Option<&str>,
        format: Option<&str>,
    ) -> MemoryResult<OntologyImportResult>;

    /// Diff two revisions of an ontology (added / removed / renamed / modified).
    async fn diff(
        &self,
        ontology_id: &str,
        from_revision: i64,
        to_revision: i64,
    ) -> MemoryResult<OntologyDiff>;

    /// Run or enqueue a label-rename migration; returns the job.
    async fn migrate(
        &self,
        ontology_id: &str,
        from_version_id: &str,
        to_version_id: &str,
        type_mappings: &[(String, String)],
        dry_run: bool,
    ) -> MemoryResult<MigrationJob>;

    /// Return one migration job's current status.
    async fn get_migration(&self, job_id: &str) -> MemoryResult<MigrationJob>;
}

/// Blanket implementation so any `&T where T: OntologyApi` also satisfies
/// [`OntologyApi`]. This lets callers pass a reference without boxing.
#[async_trait]
impl<T: OntologyApi> OntologyApi for &T {
    async fn list(&self) -> MemoryResult<Vec<OntologySummary>> {
        (*self).list().await
    }

    async fn get(&self, ontology_id: &str) -> MemoryResult<Ontology> {
        (*self).get(ontology_id).await
    }

    async fn get_active(&self) -> MemoryResult<ActiveOntology> {
        (*self).get_active().await
    }

    async fn clone(&self, template_name: &str) -> MemoryResult<OntologyVersion> {
        (*self).clone(template_name).await
    }

    async fn create(
        &self,
        name: &str,
        document: &OntologyDocument,
        validation_mode: Option<&str>,
    ) -> MemoryResult<OntologyVersion> {
        (*self).create(name, document, validation_mode).await
    }

    async fn update(
        &self,
        ontology_id: &str,
        document: &OntologyDocument,
        validation_mode: Option<&str>,
    ) -> MemoryResult<OntologyVersion> {
        (*self).update(ontology_id, document, validation_mode).await
    }

    async fn activate(&self, version_id: &str) -> MemoryResult<OntologyVersion> {
        (*self).activate(version_id).await
    }

    async fn delete(&self, ontology_id: &str) -> MemoryResult<()> {
        (*self).delete(ontology_id).await
    }

    async fn import_(
        &self,
        content: Option<&str>,
        url: Option<&str>,
        format: Option<&str>,
    ) -> MemoryResult<OntologyImportResult> {
        (*self).import_(content, url, format).await
    }

    async fn diff(
        &self,
        ontology_id: &str,
        from_revision: i64,
        to_revision: i64,
    ) -> MemoryResult<OntologyDiff> {
        (*self).diff(ontology_id, from_revision, to_revision).await
    }

    async fn migrate(
        &self,
        ontology_id: &str,
        from_version_id: &str,
        to_version_id: &str,
        type_mappings: &[(String, String)],
        dry_run: bool,
    ) -> MemoryResult<MigrationJob> {
        (*self)
            .migrate(
                ontology_id,
                from_version_id,
                to_version_id,
                type_mappings,
                dry_run,
            )
            .await
    }

    async fn get_migration(&self, job_id: &str) -> MemoryResult<MigrationJob> {
        (*self).get_migration(job_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal no-op implementation used only to verify the trait compiles
    /// and all methods are present.
    struct DummyOntology;

    #[async_trait]
    impl OntologyApi for DummyOntology {
        async fn list(&self) -> MemoryResult<Vec<OntologySummary>> {
            Ok(Vec::new())
        }

        async fn get(&self, _ontology_id: &str) -> MemoryResult<Ontology> {
            Err(muninn_core::MemoryError::not_found("dummy".to_string()))
        }

        async fn get_active(&self) -> MemoryResult<ActiveOntology> {
            Err(muninn_core::MemoryError::NotSupported("dummy".to_string()))
        }

        async fn clone(&self, _template_name: &str) -> MemoryResult<OntologyVersion> {
            Err(muninn_core::MemoryError::NotSupported("dummy".to_string()))
        }

        async fn create(
            &self,
            _name: &str,
            _document: &OntologyDocument,
            _validation_mode: Option<&str>,
        ) -> MemoryResult<OntologyVersion> {
            Err(muninn_core::MemoryError::NotSupported("dummy".to_string()))
        }

        async fn update(
            &self,
            _ontology_id: &str,
            _document: &OntologyDocument,
            _validation_mode: Option<&str>,
        ) -> MemoryResult<OntologyVersion> {
            Err(muninn_core::MemoryError::NotSupported("dummy".to_string()))
        }

        async fn activate(&self, _version_id: &str) -> MemoryResult<OntologyVersion> {
            Err(muninn_core::MemoryError::NotSupported("dummy".to_string()))
        }

        async fn delete(&self, _ontology_id: &str) -> MemoryResult<()> {
            Err(muninn_core::MemoryError::NotSupported("dummy".to_string()))
        }

        async fn import_(
            &self,
            _content: Option<&str>,
            _url: Option<&str>,
            _format: Option<&str>,
        ) -> MemoryResult<OntologyImportResult> {
            Err(muninn_core::MemoryError::NotSupported("dummy".to_string()))
        }

        async fn diff(
            &self,
            _ontology_id: &str,
            _from_revision: i64,
            _to_revision: i64,
        ) -> MemoryResult<OntologyDiff> {
            Err(muninn_core::MemoryError::NotSupported("dummy".to_string()))
        }

        async fn migrate(
            &self,
            _ontology_id: &str,
            _from_version_id: &str,
            _to_version_id: &str,
            _type_mappings: &[(String, String)],
            _dry_run: bool,
        ) -> MemoryResult<MigrationJob> {
            Err(muninn_core::MemoryError::NotSupported("dummy".to_string()))
        }

        async fn get_migration(&self, _job_id: &str) -> MemoryResult<MigrationJob> {
            Err(muninn_core::MemoryError::not_found("dummy".to_string()))
        }
    }

    #[tokio::test]
    async fn trait_compiles_and_dummy_implements_all_methods() {
        let dummy = DummyOntology;
        // list returns empty vec
        let summaries = dummy.list().await.unwrap();
        assert!(summaries.is_empty());

        // get returns not_found
        let result = dummy.get("test").await;
        assert!(result.is_err());

        // get_active returns not_supported
        let result = dummy.get_active().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn reference_blanket_impl_works() {
        let dummy = DummyOntology;
        let ref_dummy: &dyn OntologyApi = &dummy;
        let summaries = ref_dummy.list().await.unwrap();
        assert!(summaries.is_empty());
    }
}

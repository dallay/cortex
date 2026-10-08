// Shared no-op fakes for `rook-usecases` integration tests.
//
// Each file in `tests/` is a separate test target (crate), so shared helpers
// live here and are pulled in per-target with `mod common;`.
// Moved verbatim from `retry_loop_tests.rs` / `route_request_restrictions.rs`
// to remove Sonar-flagged duplication. No behavior changes.

use std::time::Duration;

use async_trait::async_trait;
use rook_core::{
    ApiFormat, AuditEntry, AuditPort, CachePort, CacheStats, CompletionRequest, CompletionResponse,
    CortexResult, FormatTranslatorPort, ModelAlias, ModelAliasRepositoryError,
    ModelAliasRepositoryPort, ModelId, ProviderId, SignatureEntry, TokenCacheStats,
};
use shared_kernel::CacheKey;

pub struct NoOpCache;

#[async_trait]
impl CachePort for NoOpCache {
    async fn get(&self, _key: &CacheKey) -> CortexResult<Option<CompletionResponse>> {
        Ok(None)
    }

    async fn set(
        &self,
        _key: &CacheKey,
        _value: &CompletionResponse,
        _ttl: Duration,
    ) -> CortexResult<()> {
        Ok(())
    }

    async fn delete(&self, _key: &CacheKey) -> CortexResult<()> {
        Ok(())
    }

    async fn clear(&self) -> CortexResult<()> {
        Ok(())
    }

    async fn stats(&self) -> CortexResult<CacheStats> {
        Ok(CacheStats {
            hits: 0,
            misses: 0,
            evictions: 0,
            entries: 0,
            max_entries: 0,
            token_cache: TokenCacheStats::default(),
        })
    }

    async fn delete_by_signature(&self, _signature: &str) -> CortexResult<usize> {
        Ok(0)
    }

    async fn list_signatures(&self) -> CortexResult<Vec<SignatureEntry>> {
        Ok(Vec::new())
    }

    async fn get_by_signature(&self, _signature: &str) -> CortexResult<Option<CompletionResponse>> {
        Ok(None)
    }

    async fn increment_token_cache_hit(&self, _tokens: u64, _cost_usd: f64) -> CortexResult<()> {
        Ok(())
    }

    async fn increment_token_cache_miss(&self) -> CortexResult<()> {
        Ok(())
    }
}

pub struct NoOpAudit;

#[async_trait]
impl AuditPort for NoOpAudit {
    async fn record(&self, _entry: AuditEntry) -> CortexResult<()> {
        Ok(())
    }
}

pub struct NoOpTranslator;

impl FormatTranslatorPort for NoOpTranslator {
    fn translate_request(
        &self,
        _from: ApiFormat,
        _to: ApiFormat,
        req: CompletionRequest,
    ) -> CortexResult<CompletionRequest> {
        Ok(req)
    }

    fn translate_response(
        &self,
        _from: ApiFormat,
        _to: ApiFormat,
        resp: CompletionResponse,
    ) -> CortexResult<CompletionResponse> {
        Ok(resp)
    }
}

/// Test stub for ModelAliasRepositoryPort
pub struct NoOpAliasRepository;

#[async_trait]
impl ModelAliasRepositoryPort for NoOpAliasRepository {
    async fn find_by_alias(
        &self,
        _alias: &ModelId,
        _provider_id: Option<&ProviderId>,
    ) -> Result<Option<ModelAlias>, ModelAliasRepositoryError> {
        Ok(None)
    }

    async fn list(&self) -> Result<Vec<ModelAlias>, ModelAliasRepositoryError> {
        Ok(vec![])
    }

    async fn create(&self, _alias: ModelAlias) -> Result<(), ModelAliasRepositoryError> {
        Ok(())
    }

    async fn delete(&self, _alias: &ModelId) -> Result<bool, ModelAliasRepositoryError> {
        Ok(false)
    }

    async fn seed(&self, _aliases: Vec<ModelAlias>) -> Result<usize, ModelAliasRepositoryError> {
        Ok(0)
    }
}

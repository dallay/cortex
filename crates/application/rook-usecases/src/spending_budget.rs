//! Strict per-key spending budget enforcement.
//!
//! The guard is the only component allowed to admit a budgeted request to the
//! provider. It proves that the selected adapter can enforce a verifiable cost
//! bound, checks that bound against the configured per-request limit, and
//! reserves the amount in the durable ledger before any upstream I/O happens.
//!
//! Invariants:
//! - No upstream I/O happens before a reservation is admitted.
//! - A reservation that already exists never authorizes a second upstream call.
//! - An unknown final cost keeps the reservation as unresolved spend. It is
//!   never treated as zero.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use rook_core::{
    ApiKeyId, CompletionRequest, ProviderPort, SpendingBudget, SpendingBudgetLedgerPort,
    SpendingBudgetReservation, SpendingBudgetReservationError, SpendingReservationOutcome,
};
use shared_kernel::{CortexError, RequestId};

/// Admission granted for one upstream attempt.
#[derive(Debug, Clone)]
pub struct SpendingAdmission {
    api_key_id: ApiKeyId,
    request_id: RequestId,
    reserved_micros_usd: u64,
}

impl SpendingAdmission {
    pub fn api_key_id(&self) -> &ApiKeyId {
        &self.api_key_id
    }

    pub fn request_id(&self) -> &RequestId {
        &self.request_id
    }

    pub fn reserved_micros_usd(&self) -> u64 {
        self.reserved_micros_usd
    }
}

/// Enforces strict spending budgets against the durable ledger.
pub struct SpendingBudgetGuard {
    ledger: Arc<dyn SpendingBudgetLedgerPort>,
}

impl SpendingBudgetGuard {
    pub fn new(ledger: Arc<dyn SpendingBudgetLedgerPort>) -> Self {
        Self { ledger }
    }

    /// Admit a request before any upstream I/O.
    ///
    /// Fails closed: without a verifiable bound, without an API key identity,
    /// or without ledger admission, no provider call is authorized.
    pub async fn admit(
        &self,
        provider: &dyn ProviderPort,
        request: &CompletionRequest,
        budget: &SpendingBudget,
        now: DateTime<Utc>,
    ) -> Result<SpendingAdmission, CortexError> {
        let api_key_id = &budget.api_key_id;
        let bound = provider
            .strict_max_cost_micros_usd(request)
            .filter(|bound| *bound > 0)
            .ok_or_else(|| {
                CortexError::cost_unavailable(
                    "no provider exposes a verifiable, enforceable request cost bound",
                )
            })?;

        if bound > budget.request_limit_micros_usd {
            return Err(CortexError::cost_unavailable(format!(
                "cost bound of {bound} micro-USD exceeds the configured per-request limit of {} micro-USD",
                budget.request_limit_micros_usd
            )));
        }

        let (window_start, window_end) = budget.window.utc_bounds(now);
        let reservation = SpendingBudgetReservation {
            api_key_id: api_key_id.clone(),
            request_id: request.id.clone(),
            window_start,
            window_end,
            reserved_micros_usd: bound,
            budget_micros_usd: budget.amount_micros_usd,
        };

        match self.ledger.reserve_spending(&reservation).await {
            Ok(SpendingReservationOutcome::Reserved) => Ok(SpendingAdmission {
                api_key_id: api_key_id.clone(),
                request_id: request.id.clone(),
                reserved_micros_usd: bound,
            }),
            Ok(SpendingReservationOutcome::Existing) => Err(CortexError::invalid_request(
                "duplicate request: the spending reservation already exists and does not authorize another upstream call",
            )),
            Err(SpendingBudgetReservationError::BudgetExceeded) => {
                Err(CortexError::spending_budget_exceeded(format!(
                    "the {} window budget of {} micro-USD is exhausted",
                    window_label(budget.window),
                    budget.amount_micros_usd
                )))
            }
            Err(error) => Err(CortexError::provider(format!(
                "spending budget ledger error: {error}"
            ))),
        }
    }

    /// Settle the reservation with the final cost, or mark it as unresolved
    /// when the final cost is unknown. Unresolved reservations keep the
    /// reserved amount charged.
    pub async fn settle(&self, admission: &SpendingAdmission, cost_micros_usd: Option<u64>) {
        if let Err(error) = self
            .ledger
            .settle_spending(
                &admission.api_key_id,
                &admission.request_id,
                cost_micros_usd,
            )
            .await
        {
            tracing::warn!(
                error = %error,
                "failed to settle the spending budget reservation; it stays charged as unresolved"
            );
        }
    }

    /// Release the reservation. Only valid when the caller can prove that no
    /// upstream I/O happened.
    pub async fn release(&self, admission: &SpendingAdmission) {
        if let Err(error) = self
            .ledger
            .release_spending_before_upstream(&admission.api_key_id, &admission.request_id)
            .await
        {
            tracing::warn!(
                error = %error,
                "failed to release the spending budget reservation; it stays charged"
            );
        }
    }
}

fn window_label(window: rook_core::SpendingBudgetWindow) -> &'static str {
    match window {
        rook_core::SpendingBudgetWindow::Daily => "daily",
        rook_core::SpendingBudgetWindow::Monthly => "monthly",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rook_core::{
        ApiFormat, CompletionResponse, HealthStatus, MessageContent, ModelId, ProviderId,
        SpendingBudgetWindow, TokenUsage,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct FakeProvider {
        id: ProviderId,
        model: ModelId,
        bound: Option<u64>,
    }

    impl FakeProvider {
        fn with_bound(bound: u64) -> Self {
            Self {
                id: ProviderId::new("fake"),
                model: ModelId::new("fake-model"),
                bound: Some(bound),
            }
        }

        fn without_bound() -> Self {
            Self {
                id: ProviderId::new("fake"),
                model: ModelId::new("fake-model"),
                bound: None,
            }
        }
    }

    #[async_trait]
    impl ProviderPort for FakeProvider {
        fn id(&self) -> &ProviderId {
            &self.id
        }

        fn supported_models(&self) -> &[ModelId] {
            std::slice::from_ref(&self.model)
        }

        fn api_format(&self) -> ApiFormat {
            ApiFormat::OpenAI
        }

        fn is_available(&self) -> bool {
            true
        }

        async fn health_check(&self) -> HealthStatus {
            HealthStatus::Healthy {
                provider: self.id.clone(),
                latency_ms: 1,
            }
        }

        fn strict_max_cost_micros_usd(&self, _req: &CompletionRequest) -> Option<u64> {
            self.bound
        }

        async fn complete(
            &self,
            req: &CompletionRequest,
        ) -> Result<CompletionResponse, CortexError> {
            Ok(CompletionResponse {
                id: req.id.clone(),
                provider: self.id.clone(),
                model: req.model.clone(),
                content: "ok".to_string(),
                content_blocks: vec![MessageContent::Text("ok".to_string())],
                thinking: None,
                tool_calls: vec![],
                finish_reason: None,
                usage: TokenUsage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    reasoning_tokens: None,
                    estimated_cost_usd: None,
                },
                latency_ms: 1,
                cache_hit: None,
            })
        }

        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> Result<
            futures::stream::BoxStream<'static, Result<rook_core::StreamChunk, CortexError>>,
            CortexError,
        > {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum LedgerAction {
        Reserve,
        Settle(Option<u64>),
        Release,
    }

    #[derive(Default)]
    struct FakeLedger {
        actions: Mutex<Vec<LedgerAction>>,
        reservations: Mutex<HashMap<(String, String), SpendingBudgetReservation>>,
        reserve_outcome:
            Mutex<Option<Result<SpendingReservationOutcome, SpendingBudgetReservationError>>>,
    }

    impl FakeLedger {
        fn with_reserve_outcome(
            outcome: Result<SpendingReservationOutcome, SpendingBudgetReservationError>,
        ) -> Self {
            Self {
                reserve_outcome: Mutex::new(Some(outcome)),
                ..Self::default()
            }
        }

        fn actions(&self) -> Vec<LedgerAction> {
            self.actions.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl SpendingBudgetLedgerPort for FakeLedger {
        async fn reserve_spending(
            &self,
            reservation: &SpendingBudgetReservation,
        ) -> Result<SpendingReservationOutcome, SpendingBudgetReservationError> {
            self.actions.lock().unwrap().push(LedgerAction::Reserve);
            if let Some(outcome) = self.reserve_outcome.lock().unwrap().take() {
                return outcome;
            }
            let key = (
                reservation.api_key_id.as_str().to_string(),
                reservation.request_id.to_string(),
            );
            self.reservations
                .lock()
                .unwrap()
                .insert(key, reservation.clone());
            Ok(SpendingReservationOutcome::Reserved)
        }

        async fn settle_spending(
            &self,
            _api_key_id: &ApiKeyId,
            _request_id: &RequestId,
            cost_micros_usd: Option<u64>,
        ) -> Result<(), SpendingBudgetReservationError> {
            self.actions
                .lock()
                .unwrap()
                .push(LedgerAction::Settle(cost_micros_usd));
            Ok(())
        }

        async fn release_spending_before_upstream(
            &self,
            _api_key_id: &ApiKeyId,
            _request_id: &RequestId,
        ) -> Result<(), SpendingBudgetReservationError> {
            self.actions.lock().unwrap().push(LedgerAction::Release);
            Ok(())
        }
    }

    fn request() -> CompletionRequest {
        CompletionRequest {
            id: RequestId::new(),
            model: ModelId::new("fake-model"),
            messages: vec![],
            stream: false,
            max_tokens: Some(100),
            temperature: None,
            tools: None,
            tool_choice: None,
            metadata: rook_core::RequestMetadata {
                origin: "test".to_string(),
                cacheable: false,
                priority: 0,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: Default::default(),
        }
    }

    fn budget() -> SpendingBudget {
        SpendingBudget {
            api_key_id: key_id(),
            amount_micros_usd: 10_000_000,
            request_limit_micros_usd: 1_000_000,
            window: SpendingBudgetWindow::Daily,
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-28T13:45:12Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn key_id() -> ApiKeyId {
        ApiKeyId::new("key-1")
    }

    #[tokio::test]
    async fn rejects_when_the_provider_cannot_demonstrate_a_cost_bound() {
        let ledger = Arc::new(FakeLedger::default());
        let guard = SpendingBudgetGuard::new(ledger.clone());

        let error = guard
            .admit(&FakeProvider::without_bound(), &request(), &budget(), now())
            .await
            .expect_err("a missing bound must fail closed");

        assert!(error.is_cost_unavailable());
        assert!(error.to_string().contains("cost unavailable"));
        assert!(
            ledger.actions().is_empty(),
            "no reservation may be created without a bound"
        );
    }

    #[tokio::test]
    async fn rejects_when_the_bound_exceeds_the_per_request_limit() {
        let ledger = Arc::new(FakeLedger::default());
        let guard = SpendingBudgetGuard::new(ledger.clone());

        let error = guard
            .admit(
                &FakeProvider::with_bound(2_000_000),
                &request(),
                &budget(),
                now(),
            )
            .await
            .expect_err("a bound above the per-request limit must fail closed");

        assert!(error.is_cost_unavailable());
        assert!(error.to_string().contains("per-request limit"));
        assert!(ledger.actions().is_empty());
    }

    #[tokio::test]
    async fn rejects_when_the_window_budget_is_exceeded() {
        let ledger = Arc::new(FakeLedger::with_reserve_outcome(Err(
            SpendingBudgetReservationError::BudgetExceeded,
        )));
        let guard = SpendingBudgetGuard::new(ledger);

        let error = guard
            .admit(
                &FakeProvider::with_bound(500_000),
                &request(),
                &budget(),
                now(),
            )
            .await
            .expect_err("an exhausted budget must fail closed");

        assert!(error.is_spending_budget_exceeded());
    }

    #[tokio::test]
    async fn rejects_an_existing_reservation_instead_of_repeating_upstream_io() {
        let ledger = Arc::new(FakeLedger::with_reserve_outcome(Ok(
            SpendingReservationOutcome::Existing,
        )));
        let guard = SpendingBudgetGuard::new(ledger);

        let error = guard
            .admit(
                &FakeProvider::with_bound(500_000),
                &request(),
                &budget(),
                now(),
            )
            .await
            .expect_err("an existing reservation must not authorize another upstream call");

        assert!(error.is_invalid_request());
        assert!(error.to_string().contains("duplicate request"));
    }

    #[tokio::test]
    async fn reserves_the_bound_inside_the_daily_utc_window() {
        let ledger = Arc::new(FakeLedger::default());
        let guard = SpendingBudgetGuard::new(ledger.clone());

        let admission = guard
            .admit(
                &FakeProvider::with_bound(500_000),
                &request(),
                &budget(),
                now(),
            )
            .await
            .expect("a bounded request is admitted");

        assert_eq!(admission.reserved_micros_usd(), 500_000);
        let reservation = ledger
            .reservations
            .lock()
            .unwrap()
            .values()
            .next()
            .cloned()
            .expect("a reservation is recorded");
        assert_eq!(reservation.reserved_micros_usd, 500_000);
        assert_eq!(reservation.budget_micros_usd, 10_000_000);
        assert_eq!(
            reservation.window_start,
            DateTime::parse_from_rfc3339("2026-10-28T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
        assert_eq!(
            reservation.window_end,
            DateTime::parse_from_rfc3339("2026-10-29T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[tokio::test]
    async fn settles_the_final_cost_when_it_is_known() {
        let ledger = Arc::new(FakeLedger::default());
        let guard = SpendingBudgetGuard::new(ledger.clone());

        let admission = guard
            .admit(
                &FakeProvider::with_bound(500_000),
                &request(),
                &budget(),
                now(),
            )
            .await
            .expect("a bounded request is admitted");
        guard.settle(&admission, Some(120_000)).await;

        assert_eq!(
            ledger.actions(),
            vec![LedgerAction::Reserve, LedgerAction::Settle(Some(120_000))]
        );
    }

    #[tokio::test]
    async fn keeps_the_reservation_unresolved_when_the_final_cost_is_unknown() {
        let ledger = Arc::new(FakeLedger::default());
        let guard = SpendingBudgetGuard::new(ledger.clone());

        let admission = guard
            .admit(
                &FakeProvider::with_bound(500_000),
                &request(),
                &budget(),
                now(),
            )
            .await
            .expect("a bounded request is admitted");
        guard.settle(&admission, None).await;

        assert_eq!(
            ledger.actions(),
            vec![LedgerAction::Reserve, LedgerAction::Settle(None)]
        );
        assert!(
            !ledger.actions().contains(&LedgerAction::Release),
            "an unknown cost must never release the reservation"
        );
    }

    #[tokio::test]
    async fn releases_the_reservation_when_no_upstream_io_happened() {
        let ledger = Arc::new(FakeLedger::default());
        let guard = SpendingBudgetGuard::new(ledger.clone());

        let admission = guard
            .admit(
                &FakeProvider::with_bound(500_000),
                &request(),
                &budget(),
                now(),
            )
            .await
            .expect("a bounded request is admitted");
        guard.release(&admission).await;

        assert_eq!(
            ledger.actions(),
            vec![LedgerAction::Reserve, LedgerAction::Release]
        );
    }
}

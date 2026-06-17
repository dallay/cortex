use std::sync::Arc;

use axum::{extract::State, Json};
use chrono::{Duration, Utc};
use rook_core::{
    ConnectionId, ProviderConnection, ProviderKind, RequestStatus, UsageFilters, UsageSummary,
};

use crate::{
    provider_dto::{
        ProviderQuotaSummaryResponse, ProviderQuotaTrendPointResponse, ProviderQuotaWindowResponse,
        ProvidersQuotaResponse,
    },
    HttpError,
};

type Usecases = Arc<rook_usecases::RookUsecases>;

pub async fn list_provider_quota(
    State(usecases): State<Usecases>,
) -> Result<Json<ProvidersQuotaResponse>, HttpError> {
    let usage_recorder = usecases.usage_recorder.as_ref().ok_or_else(|| HttpError {
        status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
        code: "USAGE_RECORDER_UNAVAILABLE",
        message: "Usage recording is not available".to_string(),
    })?;

    let connections = usecases
        .manage_connections
        .as_ref()
        .ok_or_else(|| HttpError {
            status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
            code: "PROVIDER_MANAGEMENT_UNAVAILABLE",
            message: "Provider management is not available".to_string(),
        })?
        .list()
        .await
        .map_err(|_| HttpError {
            status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL_ERROR",
            message: "internal server error".to_string(),
        })?;

    let generated_at = Utc::now();
    let last_24h_start = Some(generated_at - Duration::hours(24));
    let last_7d_start = Some(generated_at - Duration::days(7));

    let mut items = Vec::new();
    for kind in all_provider_kinds() {
        let kind_connections = connections
            .iter()
            .filter(|connection| connection.provider_kind == kind)
            .collect::<Vec<_>>();
        let connection_ids = kind_connections
            .iter()
            .map(|connection| connection.id)
            .collect::<Vec<ConnectionId>>();

        let all_time = summarize_window(usage_recorder, &connection_ids, None, None).await?;
        let last_24h =
            summarize_window(usage_recorder, &connection_ids, last_24h_start, None).await?;
        let last_7d =
            summarize_window(usage_recorder, &connection_ids, last_7d_start, None).await?;

        let trend = build_trend(usage_recorder, &connection_ids, generated_at).await?;

        let connection_count = kind_connections.len() as u32;
        let active_connection_count = kind_connections
            .iter()
            .filter(|conn| conn.is_active)
            .count() as u32;
        let (warning_threshold, error_threshold) = aggregate_thresholds(&kind_connections);
        let observed_ratio = rate_limited_ratio(&last_7d);
        let warning_level = classify_warning_level(
            observed_ratio,
            warning_threshold,
            error_threshold,
            connection_count,
        );
        let (support, note) = support_note(kind, connection_count);

        items.push(ProviderQuotaSummaryResponse {
            provider_kind: kind.as_str().to_string(),
            connection_count,
            active_connection_count,
            warning_threshold,
            error_threshold,
            support: support.to_string(),
            note: note.to_string(),
            all_time,
            last_24h,
            last_7d,
            observed_rate_limited_ratio: observed_ratio,
            warning_level: warning_level.to_string(),
            trend,
        });
    }

    Ok(Json(ProvidersQuotaResponse {
        generated_at,
        items,
    }))
}

async fn summarize_window(
    usage_recorder: &Arc<dyn rook_usecases::UsageRecorderPort>,
    connection_ids: &[ConnectionId],
    start: Option<chrono::DateTime<Utc>>,
    end: Option<chrono::DateTime<Utc>>,
) -> Result<ProviderQuotaWindowResponse, HttpError> {
    let mut total = ProviderQuotaWindowResponse::default();

    for connection_id in connection_ids {
        let filters = UsageFilters {
            connection_id: Some(*connection_id),
            start,
            end,
            ..UsageFilters::default()
        };
        let summary = usage_recorder
            .summary(filters.clone())
            .await
            .map_err(internal_error)?;
        let rate_limited_requests = usage_recorder
            .count(UsageFilters {
                status: Some(RequestStatus::RateLimited),
                ..filters.clone()
            })
            .await
            .map_err(internal_error)?;
        let cost_breakdown = usage_recorder
            .cost_breakdown(filters)
            .await
            .map_err(internal_error)?;
        let window = window_from_parts(
            summary,
            rate_limited_requests,
            cost_breakdown.total_cost_usd,
        );

        total.requests += window.requests;
        total.rate_limited_requests += window.rate_limited_requests;
        total.prompt_tokens += window.prompt_tokens;
        total.completion_tokens += window.completion_tokens;
        total.cache_read_tokens += window.cache_read_tokens;
        total.cache_creation_tokens += window.cache_creation_tokens;
        total.reasoning_tokens += window.reasoning_tokens;
        total.total_tokens += window.total_tokens;
        total.cost_usd += window.cost_usd;
    }

    Ok(total)
}

async fn build_trend(
    usage_recorder: &Arc<dyn rook_usecases::UsageRecorderPort>,
    connection_ids: &[ConnectionId],
    generated_at: chrono::DateTime<Utc>,
) -> Result<Vec<ProviderQuotaTrendPointResponse>, HttpError> {
    let mut trend = Vec::new();

    for days_ago in (0..7).rev() {
        let day_start = (generated_at - Duration::days(days_ago)).date_naive();
        let start = day_start
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| HttpError {
                status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                code: "INTERNAL_ERROR",
                message: "invalid time".to_string(),
            })?
            .and_utc();
        let end = (day_start + Duration::days(1))
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| HttpError {
                status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                code: "INTERNAL_ERROR",
                message: "invalid time".to_string(),
            })?
            .and_utc();
        let window =
            summarize_window(usage_recorder, connection_ids, Some(start), Some(end)).await?;
        trend.push(ProviderQuotaTrendPointResponse {
            date: day_start.format("%Y-%m-%d").to_string(),
            requests: window.requests,
            rate_limited_requests: window.rate_limited_requests,
            total_tokens: window.total_tokens,
            cost_usd: window.cost_usd,
        });
    }

    Ok(trend)
}

fn window_from_parts(
    summary: UsageSummary,
    rate_limited_requests: u64,
    cost_usd: f64,
) -> ProviderQuotaWindowResponse {
    let total_tokens = summary.total_prompt_tokens
        + summary.total_completion_tokens
        + summary.total_cache_read_tokens
        + summary.total_cache_creation_tokens
        + summary.total_reasoning_tokens;

    ProviderQuotaWindowResponse {
        requests: summary.total_requests,
        rate_limited_requests,
        prompt_tokens: summary.total_prompt_tokens,
        completion_tokens: summary.total_completion_tokens,
        cache_read_tokens: summary.total_cache_read_tokens,
        cache_creation_tokens: summary.total_cache_creation_tokens,
        reasoning_tokens: summary.total_reasoning_tokens,
        total_tokens,
        cost_usd,
    }
}

fn aggregate_thresholds(connections: &[&ProviderConnection]) -> (Option<f32>, Option<f32>) {
    if connections.is_empty() {
        return (None, None);
    }

    let warning = connections
        .iter()
        .map(|conn| conn.config.quota_window_thresholds.warning)
        .sum::<f32>()
        / connections.len() as f32;
    let error = connections
        .iter()
        .map(|conn| conn.config.quota_window_thresholds.error)
        .sum::<f32>()
        / connections.len() as f32;

    (Some(warning), Some(error))
}

fn rate_limited_ratio(window: &ProviderQuotaWindowResponse) -> f32 {
    if window.requests == 0 {
        0.0
    } else {
        window.rate_limited_requests as f32 / window.requests as f32
    }
}

fn classify_warning_level(
    observed_ratio: f32,
    warning_threshold: Option<f32>,
    error_threshold: Option<f32>,
    connection_count: u32,
) -> &'static str {
    if connection_count == 0 {
        return "not_configured";
    }

    match (warning_threshold, error_threshold) {
        (Some(_), Some(error)) if observed_ratio >= error => "critical",
        (Some(warning), _) if observed_ratio >= warning => "warning",
        _ => "ok",
    }
}

fn support_note(kind: ProviderKind, connection_count: u32) -> (&'static str, &'static str) {
    if connection_count == 0 {
        return (
            "not_configured",
            "No configured connections for this provider.",
        );
    }

    match kind {
        ProviderKind::OpenAI => (
            "usage_only",
            "Usage and trends come from recorded requests. OpenAI does not expose a public quota API here, so limits are inferred from local thresholds and observed usage.",
        ),
        ProviderKind::Anthropic => (
            "usage_only",
            "Usage comes from recorded requests. Anthropic workspace limits are not available through a public API in this integration.",
        ),
        ProviderKind::Gemini => (
            "usage_only",
            "Usage comes from recorded token metadata. Gemini returns token counts per request, but this integration does not have account-level remaining quota data.",
        ),
        ProviderKind::Groq => (
            "usage_only",
            "Usage comes from recorded requests and rate-limited responses. Remaining request/token quotas are not persisted yet, so this view highlights observed saturation instead.",
        ),
        ProviderKind::Ollama | ProviderKind::OllamaCloud => (
            "local_or_header_based",
            "Self-hosted Ollama does not expose a quota. For Ollama Cloud, this view shows observed usage and rate-limited responses when present.",
        ),
    }
}

fn internal_error(_error: shared_kernel::CortexError) -> HttpError {
    HttpError {
        status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        code: "INTERNAL_ERROR",
        message: "internal server error".to_string(),
    }
}

fn all_provider_kinds() -> [ProviderKind; 6] {
    [
        ProviderKind::OpenAI,
        ProviderKind::Anthropic,
        ProviderKind::Gemini,
        ProviderKind::Groq,
        ProviderKind::Ollama,
        ProviderKind::OllamaCloud,
    ]
}

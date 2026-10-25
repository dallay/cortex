use std::sync::Arc;

use axum::{extract::State, Json};
use chrono::{Duration, Utc};
use rook_core::{
    ConnectionId, ProviderConnection, ProviderKind, RequestStatus, UsageFilters, UsageSummary,
};

use crate::{
    provider_dto::{
        ProviderAccountQuotaResponse, ProviderQuotaSummaryResponse, ProviderQuotaThresholdResponse,
        ProviderQuotaTrendPointResponse, ProviderQuotaWindowResponse,
        ProviderRateLimitObservationResponse, ProvidersQuotaResponse,
    },
    HttpError,
};

type Usecases = Arc<rook_usecases::RookUsecases>;

pub async fn list_provider_quota(
    State(usecases): State<Usecases>,
) -> Result<Json<ProvidersQuotaResponse>, HttpError> {
    let recorder = usecases.usage_recorder.as_ref().ok_or_else(|| HttpError {
        status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
        code: "USAGE_RECORDER_UNAVAILABLE",
        message: "Usage recording is not available".to_string(),
    })?;
    let manage_connections = usecases
        .manage_connections
        .as_ref()
        .ok_or_else(|| HttpError {
            status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
            code: "PROVIDER_MANAGEMENT_UNAVAILABLE",
            message: "Provider management is not available".to_string(),
        })?;
    let connections = manage_connections.list().await.map_err(|error| {
        tracing::warn!(%error, "failed to list provider connections for quota summary");
        HttpError {
            status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL_ERROR",
            message: "internal server error".to_string(),
        }
    })?;
    let generated_at = Utc::now();
    let last_24h_start = Some(generated_at - Duration::hours(24));
    let last_7d_start = Some(generated_at - Duration::days(7));
    let mut items = Vec::new();

    for kind in all_provider_kinds() {
        let item = summarize_provider_kind(
            recorder,
            &connections,
            kind,
            generated_at,
            last_24h_start,
            last_7d_start,
        )
        .await;
        items.push(item);
    }
    Ok(Json(ProvidersQuotaResponse {
        generated_at,
        items,
    }))
}

async fn summarize_provider_kind(
    recorder: &Arc<dyn rook_usecases::UsageRecorderPort>,
    connections: &[ProviderConnection],
    kind: ProviderKind,
    generated_at: chrono::DateTime<Utc>,
    last_24h_start: Option<chrono::DateTime<Utc>>,
    last_7d_start: Option<chrono::DateTime<Utc>>,
) -> ProviderQuotaSummaryResponse {
    let provider_connections = connections
        .iter()
        .filter(|c| c.provider_kind == kind)
        .collect::<Vec<_>>();
    let ids = provider_connections
        .iter()
        .map(|c| c.id)
        .collect::<Vec<_>>();
    let (all_time, last_24h, last_7d, trend) = if ids.is_empty() {
        (
            empty_window(),
            empty_window(),
            empty_window(),
            empty_trend(generated_at),
        )
    } else {
        (
            summarize_window(recorder, &ids, None, None).await,
            summarize_window(recorder, &ids, last_24h_start, None).await,
            summarize_window(recorder, &ids, last_7d_start, None).await,
            build_trend(recorder, &ids, generated_at)
                .await
                .unwrap_or_default(),
        )
    };
    let connection_count = provider_connections.len() as u32;
    let active_connection_count =
        provider_connections.iter().filter(|c| c.is_active).count() as u32;
    let has_available_rate_limit =
        last_7d.rate_limit_availability == "available" && last_7d.availability != "unavailable";
    let ratio = if has_available_rate_limit {
        rate_limited_ratio(&last_7d)
    } else {
        None
    };
    let connection_ratios = if has_available_rate_limit {
        connection_rate_limit_ratios(recorder, &provider_connections, last_7d_start).await
    } else {
        vec![None; provider_connections.len()]
    };
    let local_warning_level = classify_warning_level(&provider_connections, &connection_ratios);

    ProviderQuotaSummaryResponse {
        provider_kind: kind.as_str().to_string(),
        connection_count,
        active_connection_count,
        coverage: "cortex_routed",
        usage_availability: all_time.availability,
        account_quota: ProviderAccountQuotaResponse {
            availability: if kind == ProviderKind::Ollama {
                "not_applicable"
            } else {
                "not_provided"
            },
        },
        local_thresholds: grouped_thresholds(&provider_connections),
        rate_limit_observation: ProviderRateLimitObservationResponse {
            source: "recorded_routed_request_status",
            availability: if ratio.is_some() {
                "available"
            } else {
                "unavailable"
            },
            window: "last_7d",
            rate_limited_requests: if ratio.is_some() {
                last_7d.rate_limited_requests
            } else {
                None
            },
            requests: if ratio.is_some() {
                last_7d.requests
            } else {
                None
            },
            ratio,
        },
        local_warning_level,
        all_time,
        last_24h,
        last_7d,
        trend,
    }
}

async fn summarize_window(
    recorder: &Arc<dyn rook_usecases::UsageRecorderPort>,
    ids: &[ConnectionId],
    start: Option<chrono::DateTime<Utc>>,
    end: Option<chrono::DateTime<Utc>>,
) -> ProviderQuotaWindowResponse {
    let mut total = empty_window();
    let mut cost_sum = 0.0;
    let mut cost_unavailable = false;
    for id in ids {
        let filters = UsageFilters {
            connection_id: Some(*id),
            start,
            end,
            ..UsageFilters::default()
        };
        let summary = recorder.summary(filters.clone()).await;
        let rate_count = recorder
            .count(UsageFilters {
                status: Some(RequestStatus::RateLimited),
                ..filters.clone()
            })
            .await;
        let request_count = recorder.count(filters.clone()).await;
        let cost = recorder.cost_breakdown(filters).await;

        match summary {
            Ok(summary) => {
                let w = window_from_parts(summary, 0, 0.0, 0, 0);
                total.availability = merge_usage_availability(total.availability, w.availability);
                total.requests = add_optional(total.requests, w.requests);
                total.prompt_tokens = add_optional(total.prompt_tokens, w.prompt_tokens);
                total.completion_tokens =
                    add_optional(total.completion_tokens, w.completion_tokens);
                total.cache_read_tokens =
                    add_optional(total.cache_read_tokens, w.cache_read_tokens);
                total.cache_creation_tokens =
                    add_optional(total.cache_creation_tokens, w.cache_creation_tokens);
                total.reasoning_tokens = add_optional(total.reasoning_tokens, w.reasoning_tokens);
                total.total_tokens = add_optional(total.total_tokens, w.total_tokens);
            }
            Err(error) => {
                tracing::warn!(%error, connection_id = %id, "failed to summarize provider usage");
                total.availability = "unavailable";
                total.requests = None;
                total.prompt_tokens = None;
                total.completion_tokens = None;
                total.cache_read_tokens = None;
                total.cache_creation_tokens = None;
                total.reasoning_tokens = None;
                total.total_tokens = None;
            }
        }

        match (request_count, rate_count) {
            (Ok(requests), Ok(limited)) if limited <= requests => {
                total.rate_limited_requests =
                    add_optional(total.rate_limited_requests, Some(limited));
            }
            (request_count, rate_count) => {
                if let Err(error) = request_count.as_ref() {
                    tracing::warn!(%error, connection_id = %id, "failed to count provider requests");
                }
                if let Err(error) = rate_count.as_ref() {
                    tracing::warn!(%error, connection_id = %id, "failed to count rate-limited provider requests");
                }
                total.rate_limit_availability = "unavailable";
                total.rate_limited_requests = None;
            }
        }

        match cost {
            Ok(cost) => {
                let window_availability =
                    if cost.known_cost_entries + cost.unknown_cost_entries == 0 {
                        "no_data"
                    } else if cost.known_cost_entries == 0 {
                        "unknown"
                    } else if cost.unknown_cost_entries > 0 {
                        "partial"
                    } else {
                        "complete"
                    };
                total.cost_availability =
                    combine_cost_availability(total.cost_availability, window_availability);
                if window_availability == "complete" || window_availability == "partial" {
                    cost_sum += cost.total_cost_usd;
                }
            }
            Err(error) => {
                tracing::warn!(%error, connection_id = %id, "failed to retrieve provider cost breakdown");
                cost_unavailable = true;
                total.cost_availability = "unavailable";
            }
        }
    }
    total.cost_usd = aggregate_cost_result(total.cost_availability, cost_sum, cost_unavailable);
    if total.availability == "unavailable" {
        total.requests = None;
        total.prompt_tokens = None;
        total.completion_tokens = None;
        total.cache_read_tokens = None;
        total.cache_creation_tokens = None;
        total.reasoning_tokens = None;
        total.total_tokens = None;
    }
    if total.rate_limit_availability == "unavailable" {
        total.rate_limited_requests = None;
    }
    total
}

async fn connection_rate_limit_ratios(
    recorder: &Arc<dyn rook_usecases::UsageRecorderPort>,
    connections: &[&ProviderConnection],
    start: Option<chrono::DateTime<Utc>>,
) -> Vec<Option<f32>> {
    let mut ratios = Vec::with_capacity(connections.len());
    for connection in connections {
        let filters = UsageFilters {
            connection_id: Some(connection.id),
            start,
            ..UsageFilters::default()
        };
        let requests = recorder.count(filters.clone()).await;
        let limited = recorder
            .count(UsageFilters {
                status: Some(RequestStatus::RateLimited),
                ..filters
            })
            .await;
        match (&requests, &limited) {
            (Err(error), _) => {
                tracing::warn!(%error, connection_id = %connection.id, "failed to count provider requests for rate-limit ratio")
            }
            (_, Err(error)) => {
                tracing::warn!(%error, connection_id = %connection.id, "failed to count rate-limited provider requests for rate-limit ratio")
            }
            _ => {}
        }
        ratios.push(match (limited, requests) {
            (Ok(limited), Ok(requests)) if requests > 0 && limited <= requests => {
                Some(limited as f32 / requests as f32)
            }
            _ => None,
        });
    }
    ratios
}

fn aggregate_cost_result(availability: &str, cost_sum: f64, lookup_failed: bool) -> Option<f64> {
    if lookup_failed {
        None
    } else {
        cost_for_availability(availability, cost_sum)
    }
}

fn cost_for_availability(availability: &str, cost_sum: f64) -> Option<f64> {
    (availability == "complete").then_some(cost_sum)
}

fn add_optional(current: Option<u64>, next: Option<u64>) -> Option<u64> {
    current.zip(next).map(|(left, right)| left + right)
}

fn merge_usage_availability(current: &'static str, next: &'static str) -> &'static str {
    match (current, next) {
        ("unavailable", _) | (_, "unavailable") => "unavailable",
        ("available", _) | (_, "available") => "available",
        _ => "no_data",
    }
}

fn empty_window() -> ProviderQuotaWindowResponse {
    ProviderQuotaWindowResponse {
        availability: "no_data",
        rate_limit_availability: "available",
        requests: Some(0),
        rate_limited_requests: Some(0),
        prompt_tokens: Some(0),
        completion_tokens: Some(0),
        cache_read_tokens: Some(0),
        cache_creation_tokens: Some(0),
        reasoning_tokens: Some(0),
        total_tokens: Some(0),
        cost_usd: None,
        cost_availability: "no_data",
    }
}

fn empty_trend(generated_at: chrono::DateTime<Utc>) -> Vec<ProviderQuotaTrendPointResponse> {
    (0..7)
        .rev()
        .map(|days_ago| {
            let date = (generated_at - Duration::days(days_ago)).date_naive();
            ProviderQuotaTrendPointResponse {
                date: date.format("%Y-%m-%d").to_string(),
                usage_availability: "no_data",
                requests: Some(0),
                rate_limited_requests: Some(0),
                total_tokens: Some(0),
                cost_usd: None,
                cost_availability: "no_data",
            }
        })
        .collect()
}

async fn build_trend(
    recorder: &Arc<dyn rook_usecases::UsageRecorderPort>,
    ids: &[ConnectionId],
    generated: chrono::DateTime<Utc>,
) -> Result<Vec<ProviderQuotaTrendPointResponse>, HttpError> {
    let mut points = Vec::new();
    for ago in (0..7).rev() {
        let day = (generated - Duration::days(ago)).date_naive();
        let start = day.and_hms_opt(0, 0, 0).expect("midnight").and_utc();
        let end = (day + Duration::days(1))
            .and_hms_opt(0, 0, 0)
            .expect("midnight")
            .and_utc();
        let w = summarize_window(recorder, ids, Some(start), Some(end)).await;
        points.push(ProviderQuotaTrendPointResponse {
            date: day.format("%Y-%m-%d").to_string(),
            usage_availability: w.availability,
            requests: w.requests,
            rate_limited_requests: w.rate_limited_requests,
            total_tokens: w.total_tokens,
            cost_usd: w.cost_usd,
            cost_availability: w.cost_availability,
        });
    }
    Ok(points)
}

fn window_from_parts(
    summary: UsageSummary,
    rate_limited: u64,
    cost: f64,
    known: u64,
    unknown: u64,
) -> ProviderQuotaWindowResponse {
    let tokens = summary.total_prompt_tokens
        + summary.total_completion_tokens
        + summary.total_cache_read_tokens
        + summary.total_cache_creation_tokens
        + summary.total_reasoning_tokens;
    let cost_availability = if summary.total_requests == 0 {
        "no_data"
    } else if known == 0 {
        "unknown"
    } else if unknown > 0 {
        "partial"
    } else {
        "complete"
    };
    ProviderQuotaWindowResponse {
        availability: if summary.total_requests == 0 {
            "no_data"
        } else {
            "available"
        },
        rate_limit_availability: "available",
        requests: Some(summary.total_requests),
        rate_limited_requests: Some(rate_limited),
        prompt_tokens: Some(summary.total_prompt_tokens),
        completion_tokens: Some(summary.total_completion_tokens),
        cache_read_tokens: Some(summary.total_cache_read_tokens),
        cache_creation_tokens: Some(summary.total_cache_creation_tokens),
        reasoning_tokens: Some(summary.total_reasoning_tokens),
        total_tokens: Some(tokens),
        cost_usd: (cost_availability == "complete").then_some(cost),
        cost_availability,
    }
}

fn combine_cost_availability(a: &str, b: &str) -> &'static str {
    match (a, b) {
        ("unavailable", _) | (_, "unavailable") => "unavailable",
        ("partial", _) | (_, "partial") => "partial",
        ("unknown", "complete") | ("complete", "unknown") => "partial",
        ("unknown", "unknown") => "unknown",
        ("complete", "no_data") | ("no_data", "complete") => "complete",
        ("unknown", "no_data") | ("no_data", "unknown") => "unknown",
        ("no_data", "no_data") => "no_data",
        ("complete", "complete") => "complete",
        _ => "partial",
    }
}

fn rate_limited_ratio(w: &ProviderQuotaWindowResponse) -> Option<f32> {
    match (w.rate_limited_requests, w.requests) {
        (Some(limited), Some(requests)) if requests > 0 => Some(limited as f32 / requests as f32),
        _ => None,
    }
}
fn grouped_thresholds(conns: &[&ProviderConnection]) -> Vec<ProviderQuotaThresholdResponse> {
    let mut out: Vec<ProviderQuotaThresholdResponse> = vec![];
    for c in conns {
        let t = &c.config.quota_window_thresholds;
        if let Some(x) = out
            .iter_mut()
            .find(|x| x.warning == t.warning && x.error == t.error)
        {
            x.connection_count += 1
        } else {
            out.push(ProviderQuotaThresholdResponse {
                warning: t.warning,
                error: t.error,
                connection_count: 1,
            })
        }
    }
    out
}
fn classify_warning_level(conns: &[&ProviderConnection], ratios: &[Option<f32>]) -> &'static str {
    if conns.is_empty() {
        return "not_configured";
    }
    let mut result = "ok";
    let mut observed = false;
    let mut missing = false;
    for (connection, ratio) in conns.iter().zip(ratios) {
        let Some(ratio) = ratio else {
            missing = true;
            continue;
        };
        observed = true;
        let thresholds = &connection.config.quota_window_thresholds;
        if *ratio >= thresholds.error {
            return "critical";
        }
        if *ratio >= thresholds.warning {
            result = "warning";
        }
    }
    if !observed || missing && result == "ok" {
        "unknown"
    } else {
        result
    }
}
fn all_provider_kinds() -> [ProviderKind; 7] {
    [
        ProviderKind::OpenAI,
        ProviderKind::Anthropic,
        ProviderKind::Gemini,
        ProviderKind::Groq,
        ProviderKind::Ollama,
        ProviderKind::OllamaCloud,
        ProviderKind::AzureOpenAI,
    ]
}

// server — HTTP server bootstrap

use std::net::SocketAddr;

use anyhow::Context;
use tokio::net::TcpListener;
use tracing::info;

use super::config::ServerConfig;
use super::di::RookContainer;
use crate::usage_retention::{
    run_startup_usage_retention_sweep, spawn_periodic_usage_retention_sweep,
};

pub async fn run(container: RookContainer, config: ServerConfig) -> anyhow::Result<()> {
    // Run startup retention sweep BEFORE accepting traffic.
    // Failures warn but do not prevent server start.
    let startup_deleted = run_startup_usage_retention_sweep(
        &container.usage_repository,
        container.usage_config.retention_days,
    )
    .await
    .inspect_err(|e| {
        tracing::warn!(error = %e, "startup usage retention sweep failed — continuing without it");
    })
    .unwrap_or(0);
    if startup_deleted > 0 {
        tracing::info!(
            deleted_rows = startup_deleted,
            "startup retention sweep completed before traffic"
        );
    }

    // Spawn periodic retention sweep in the background.
    let _periodic_sweep = spawn_periodic_usage_retention_sweep(
        container.usage_repository.clone(),
        container.usage_config.retention_days,
        container.usage_config.sweep_interval_hours,
    );

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .context("invalid address")?;

    let router = transport_axum::router(
        container.usecases.clone(),
        container.authz_config.clone(),
        container.login_rate_limiter.clone(),
        container.ip_rate_limiter.clone(),
        container.api_key_rate_limiter.clone(),
        container.csrf_guard.clone(),
        container.rate_limit_store.clone(),
    )
    .merge(crate::dashboard::dashboard_routes());

    info!(addr = %addr, "starting rook server");

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}

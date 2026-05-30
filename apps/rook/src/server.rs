// server — HTTP server bootstrap

use std::net::SocketAddr;

use anyhow::Context;
use tokio::net::TcpListener;
use tracing::info;

use super::config::ServerConfig;
use super::di::RookContainer;

pub async fn run(container: RookContainer, config: ServerConfig) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .context("invalid address")?;

    let router = transport_axum::router(
        container.usecases.clone(),
        container.authz_config.clone(),
        container.login_rate_limiter.clone(),
        container.api_key_rate_limiter.clone(),
        container.csrf_guard.clone(),
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

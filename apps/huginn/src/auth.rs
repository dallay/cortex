//! Standalone connection composition: no session store, model, tools, or chat loop.
use anyhow::Context;
use huginn::config::Config;
use huginn_application::{
    connections::{ConnectionAccess, ConnectionManager, ConnectionsPlugin},
    providers::{catalog_id, providers_id, ProvidersPlugin},
};
use huginn_core::{
    kernel::Supervisor,
    providers::{ProviderCatalogService, ProvidersService},
};
use huginn_runtime::connections::{HostSecrets, SqliteConnections};
use std::{io::IsTerminal, sync::Arc};

pub fn require_interactive(json: bool, line_mode: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal() && !json && !line_mode,
        "Provider login requires an interactive terminal. Run `huginn auth login` directly in a terminal without --json, --line-mode, or input/output redirection. Credentials cannot be supplied as arguments or piped input."
    );
    Ok(())
}

pub async fn login(config: Config, workspace: Option<std::path::PathBuf>) -> anyhow::Result<()> {
    let access = ConnectionAccess::new(Arc::new(HostSecrets::default()));
    let mut supervisor = Supervisor::default();
    let result = async {
        supervisor.register(Box::new(ProvidersPlugin::with_credentials(access.clone())))?;
        supervisor.register(Box::new(huginn_runtime::providers::OllamaLocalPlugin { timeout_secs: config.timeout_secs }))?;
        supervisor.register(Box::new(huginn_runtime::providers::OllamaCloudPlugin { timeout_secs: config.timeout_secs }))?;
        supervisor.register(Box::new(huginn_runtime::providers::CustomProviderPlugin { timeout_secs: config.timeout_secs }))?;
        supervisor.start().await?;
        let providers = supervisor.resolve::<ProvidersService>(&providers_id())?;
        let catalog = supervisor.resolve::<ProviderCatalogService>(&catalog_id())?;
        let store = Arc::new(SqliteConnections::open(
            &config.db.with_extension("providers.db"),
        ).context("opening secure connection storage")?);
        let manager = Arc::new(ConnectionManager::open(store, access, providers.0.clone(), catalog.0.clone()).await?);
        supervisor.register(Box::new(ConnectionsPlugin(manager.clone())))?;
        supervisor.start().await?;
        let saved = Arc::new(std::sync::atomic::AtomicBool::new(false));
        supervisor.register(Box::new(huginn_presentation::ContributionsPlugin))?;
        supervisor.register(Box::new(huginn_presentation::providers::StandaloneProvidersUiPlugin(saved.clone())))?;
        supervisor.start().await?;
        let contributions = supervisor.resolve::<huginn_presentation::ProviderUiService>(&huginn_presentation::contributions_id())?;
        let saved = huginn_presentation::login(contributions.0.clone(), saved, workspace.unwrap_or(std::env::current_dir()?)).await?;
        if saved {
            println!("Connection configured locally.\nNext: select a model with the existing /model workflow.");
            Ok(())
        } else {
            anyhow::bail!("Provider login cancelled. No successful save was confirmed; if a write was pending, check connection status on the next startup.")
        }
    }.await;
    supervisor.shutdown().await;
    result
}

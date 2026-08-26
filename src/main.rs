use std::{net::SocketAddr, sync::Arc};

use agentbox_mcp::{
    auth::{AuthLayer, AuthThrottle},
    bootstrap::Bootstrapper,
    config::{Cli, Config},
    exec::ProcessManager,
    mcp::{AppState, build_router},
    mcp_proxy::McpProxyRegistry,
    skills::SkillCatalog,
};
use anyhow::Context;
use axum::serve;
use clap::Parser;
use tokio::{net::TcpListener, signal};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();
    let config = Arc::new(Config::load(cli.config.as_deref())?);
    config.warn_if_insecure_auth();

    let bind: SocketAddr = config
        .server
        .bind
        .parse()
        .with_context(|| format!("invalid bind address {}", config.server.bind))?;

    let manager = Arc::new(ProcessManager::new(config.exec.clone()));
    let auth = Arc::new(AuthLayer::new(config.auth.clone()).await?);
    let throttle = Arc::new(AuthThrottle::new());
    let skills = Arc::new(SkillCatalog::new(config.skills.clone()));
    let bootstrap = Arc::new(Bootstrapper::new(config.clone()));
    let mcp_proxy = Arc::new(McpProxyRegistry::connect(&config.mcp_proxy).await);
    let state = AppState {
        config: config.clone(),
        manager,
        auth,
        throttle,
        skills,
        bootstrap,
        mcp_proxy,
        fake_oauth_codes: Arc::new(Default::default()),
    };

    let app = build_router(state).layer(TraceLayer::new_for_http());
    let listener = TcpListener::bind(bind).await?;
    tracing::info!(%bind, "agentbox-mcp listening");
    serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
        .with_graceful_shutdown(async {
            let _ = signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

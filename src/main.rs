use std::sync::Arc;

use myagentrust::{
    AppConfig, AppState, create_app, db::create_pool, solanasetup::create_rpc_client,
};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Arc::new(AppConfig::from_env());
    let database_url = std::env::var("DATABASE_URL")?;
    let pool = create_pool(&database_url).await?;
    let rpc_url = std::env::var("SOLANA_RPC_URL")?;
    let rpc = create_rpc_client(&rpc_url);

    sqlx::migrate!().run(&pool).await?;

    let state = AppState {
        db: pool,
        rpc,
        http: reqwest::Client::new(),
        config,
    };
    let app = create_app(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:9000").await?;

    axum::serve(listener, app).await?;

    Ok(())
}

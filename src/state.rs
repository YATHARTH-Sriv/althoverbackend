use std::sync::Arc;

use solana_client::nonblocking::rpc_client::RpcClient;
use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub rpc: Arc<RpcClient>,
    pub http: reqwest::Client,
}

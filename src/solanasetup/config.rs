use std::sync::Arc;

use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::CommitmentConfig};

pub fn create_rpc_client(rpc_url: &str) -> Arc<RpcClient> {
    Arc::new(RpcClient::new_with_commitment(
        rpc_url.to_string(),
        CommitmentConfig::confirmed(),
    ))
}

use borsh::BorshDeserialize;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;

use crate::solanasetup::{PROGRAM_ID, anchor_discriminator, derive_program_config_pda};

#[derive(Debug, BorshDeserialize)]
pub struct ProgramConfig {
    pub smart_account_index: u128,
    pub authority: Pubkey,
    pub smart_account_creation_fee: u64,
    pub treasury: Pubkey,
    pub reserved: [u8; 64],
}

pub fn decode_program_config(owner: &Pubkey, data: &[u8]) -> Result<ProgramConfig, String> {
    if owner != &PROGRAM_ID {
        return Err("ProgramConfig has an unexpected owner".into());
    }

    if data.len() < 8 {
        return Err("ProgramConfig data is too short".into());
    }

    let expected = anchor_discriminator("account:ProgramConfig");

    if data[..8] != expected {
        return Err("Invalid ProgramConfig discriminator".into());
    }

    ProgramConfig::try_from_slice(&data[8..])
        .map_err(|error| format!("Invalid ProgramConfig data: {error}"))
}

pub async fn fetch_program_config(rpc: &RpcClient) -> Result<(Pubkey, ProgramConfig), String> {
    let (program_config_pda, _) = derive_program_config_pda();

    let account = rpc
        .get_account(&program_config_pda)
        .await
        .map_err(|error| format!("Failed to fetch ProgramConfig: {error}"))?;

    let config = decode_program_config(&account.owner, &account.data)?;

    Ok((program_config_pda, config))
}

use sha2::{Digest, Sha256};
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use solana_system_interface::instruction as system_instruction;

use crate::{
    ApiError, AppState, bad_request, internal_error,
    solanasetup::{build_unsigned_transaction_base64, verify_confirmed_sol_transfer},
};

pub(super) fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub(super) fn generate_public_token() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(internal_error)?;
    Ok(hex::encode(bytes))
}

pub(super) async fn build_transfer(
    state: &AppState,
    payer: Pubkey,
    recipient: Pubkey,
    amount_lamports: u64,
) -> Result<String, ApiError> {
    let blockhash = state
        .rpc
        .get_latest_blockhash()
        .await
        .map_err(internal_error)?;
    let instruction = system_instruction::transfer(&payer, &recipient, amount_lamports);
    build_unsigned_transaction_base64(payer, blockhash, instruction).map_err(internal_error)
}

pub(super) async fn verify_transfer(
    state: &AppState,
    signature: &Signature,
    payer: &Pubkey,
    recipient: &Pubkey,
    amount_lamports: u64,
) -> Result<(), ApiError> {
    verify_confirmed_sol_transfer(
        state.rpc.as_ref(),
        signature,
        payer,
        recipient,
        amount_lamports,
    )
    .await
    .map_err(bad_request)
}

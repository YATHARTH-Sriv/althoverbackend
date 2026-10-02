use std::str::FromStr;

use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signature::Signature};

use crate::{
    ApiError, AppState, bad_request, internal_error,
    solanasetup::{
        PROGRAM_ID, build_unsigned_transaction_base64, derive_settings_pda,
        verify_confirmed_transaction,
    },
    squadsaccounts::{SettingsAccount, decode_settings_account},
};

pub(super) async fn fetch_chain_settings(
    state: &AppState,
    settings_pda: &Pubkey,
) -> Result<SettingsAccount, ApiError> {
    let account = state
        .rpc
        .get_account(settings_pda)
        .await
        .map_err(internal_error)?;
    let settings =
        decode_settings_account(&account.owner, &account.data).map_err(internal_error)?;

    if derive_settings_pda(settings.seed).0 != *settings_pda {
        return Err(bad_request(
            "Settings account does not match its on-chain seed",
        ));
    }

    Ok(settings)
}

pub(super) fn ensure_authority(
    settings: &SettingsAccount,
    wallet_address: &Pubkey,
) -> Result<(), ApiError> {
    if settings.settings_authority != *wallet_address {
        return Err(bad_request(
            "Only the settings authority can perform this action directly",
        ));
    }
    Ok(())
}

pub(super) async fn build_unsigned_settings_transaction(
    state: &AppState,
    authority: Pubkey,
    instruction: Instruction,
) -> Result<String, ApiError> {
    let blockhash = state
        .rpc
        .get_latest_blockhash()
        .await
        .map_err(internal_error)?;
    build_unsigned_transaction_base64(authority, blockhash, instruction).map_err(internal_error)
}

pub(super) async fn verify_submitted_transaction(
    state: &AppState,
    tx_sig: &str,
    authority: &Pubkey,
    settings_pda: &Pubkey,
) -> Result<Signature, ApiError> {
    let signature =
        Signature::from_str(tx_sig.trim()).map_err(|_| bad_request("txSig is invalid"))?;

    verify_confirmed_transaction(
        state.rpc.as_ref(),
        &signature,
        authority,
        &[*settings_pda, PROGRAM_ID],
    )
    .await
    .map_err(bad_request)?;

    Ok(signature)
}

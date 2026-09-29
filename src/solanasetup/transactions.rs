use base64::{Engine as _, engine::general_purpose::STANDARD};
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{CommitmentConfig, RpcTransactionConfig},
};
use solana_sdk::{
    hash::Hash, instruction::Instruction, message::Message, pubkey::Pubkey, signature::Signature,
    transaction::Transaction,
};
use solana_transaction_status_client_types::UiTransactionEncoding;

use crate::solanasetup::PROGRAM_ID;
use crate::solanasetup::anchor_discriminator;

pub fn build_unsigned_transaction_base64(
    fee_payer: Pubkey,
    recent_blockhash: Hash,
    instruction: Instruction,
) -> Result<String, String> {
    let message = Message::new_with_blockhash(&[instruction], Some(&fee_payer), &recent_blockhash);

    let transaction = Transaction::new_unsigned(message);

    let serialized = bincode::serialize(&transaction)
        .map_err(|error| format!("Failed to serialize transaction: {error}"))?;

    Ok(STANDARD.encode(serialized))
}

pub fn decode_signed_transaction(transaction_base64: &str) -> Result<Transaction, String> {
    if transaction_base64.len() > 4096 {
        return Err("Transaction payload is too large".to_owned());
    }

    let bytes = STANDARD
        .decode(transaction_base64.trim())
        .map_err(|_| "Transaction is not valid Base64".to_owned())?;

    let transaction: Transaction = bincode::deserialize(&bytes)
        .map_err(|error| format!("Invalid signed transaction: {error}"))?;

    transaction
        .verify()
        .map_err(|error| format!("Transaction signature verification failed: {error}"))?;

    Ok(transaction)
}

pub async fn verify_confirmed_transaction(
    rpc: &RpcClient,
    signature: &Signature,
    expected_signer: &Pubkey,
    expected_accounts: &[Pubkey],
) -> Result<(), String> {
    verify_confirmed_transaction_inner(rpc, signature, expected_signer, expected_accounts, None)
        .await
}

pub async fn verify_confirmed_squads_instruction(
    rpc: &RpcClient,
    signature: &Signature,
    expected_signer: &Pubkey,
    expected_accounts: &[Pubkey],
    instruction_name: &str,
) -> Result<(), String> {
    verify_confirmed_transaction_inner(
        rpc,
        signature,
        expected_signer,
        expected_accounts,
        Some(anchor_discriminator(instruction_name)),
    )
    .await
}

async fn verify_confirmed_transaction_inner(
    rpc: &RpcClient,
    signature: &Signature,
    expected_signer: &Pubkey,
    expected_accounts: &[Pubkey],
    expected_discriminator: Option<[u8; 8]>,
) -> Result<(), String> {
    let confirmed = rpc
        .confirm_transaction_with_commitment(signature, CommitmentConfig::confirmed())
        .await
        .map_err(|error| format!("Failed to confirm transaction: {error}"))?;

    if !confirmed.value {
        return Err("Transaction is not confirmed".to_owned());
    }

    let confirmed_transaction = rpc
        .get_transaction_with_config(
            signature,
            RpcTransactionConfig {
                encoding: Some(UiTransactionEncoding::Base64),
                commitment: Some(CommitmentConfig::confirmed()),
                max_supported_transaction_version: Some(0),
            },
        )
        .await
        .map_err(|error| format!("Unable to retrieve confirmed transaction: {error}"))?;

    let metadata = confirmed_transaction
        .transaction
        .meta
        .ok_or_else(|| "Confirmed transaction has no metadata".to_owned())?;

    if let Some(error) = metadata.err {
        return Err(format!("Transaction failed on-chain: {error:?}"));
    }

    let transaction = confirmed_transaction
        .transaction
        .transaction
        .decode()
        .ok_or_else(|| "Unable to decode confirmed transaction".to_owned())?;

    let account_keys = transaction.message.static_account_keys();
    let required_signatures = usize::from(transaction.message.header().num_required_signatures);

    let signer_keys = account_keys
        .get(..required_signatures)
        .ok_or_else(|| "Invalid transaction signer metadata".to_owned())?;

    if !signer_keys.contains(expected_signer) {
        return Err("Transaction was not signed by the expected wallet".to_owned());
    }

    for expected_account in expected_accounts {
        if !account_keys.contains(expected_account) {
            return Err(format!(
                "Transaction does not reference expected account \
                 {expected_account}"
            ));
        }
    }

    if let Some(discriminator) = expected_discriminator {
        let invoked_expected_instruction =
            transaction
                .message
                .instructions()
                .iter()
                .any(|instruction| {
                    account_keys.get(usize::from(instruction.program_id_index)) == Some(&PROGRAM_ID)
                        && instruction.data.starts_with(&discriminator)
                });
        if !invoked_expected_instruction {
            return Err("Transaction does not invoke the expected Squads instruction".to_owned());
        }
    }

    Ok(())
}

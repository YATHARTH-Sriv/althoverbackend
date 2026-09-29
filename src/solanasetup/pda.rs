use solana_sdk::pubkey::Pubkey;

use crate::solanasetup::{
    PROGRAM_ID, SEED_PREFIX, SEED_PROGRAM_CONFIG, SEED_PROPOSAL, SEED_SETTINGS, SEED_SMART_ACCOUNT,
    SEED_TRANSACTION,
};

pub fn derive_program_config_pda() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SEED_PREFIX, SEED_PROGRAM_CONFIG], &PROGRAM_ID)
}

pub fn derive_transaction_pda(settings_pda: &Pubkey, transaction_index: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            SEED_PREFIX,
            settings_pda.as_ref(),
            SEED_TRANSACTION,
            &transaction_index.to_le_bytes(),
        ],
        &PROGRAM_ID,
    )
}

pub fn derive_proposal_pda(settings_pda: &Pubkey, transaction_index: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            SEED_PREFIX,
            settings_pda.as_ref(),
            SEED_TRANSACTION,
            &transaction_index.to_le_bytes(),
            SEED_PROPOSAL,
        ],
        &PROGRAM_ID,
    )
}

pub fn derive_settings_pda(settings_seed: u128) -> (Pubkey, u8) {
    let seed_bytes = settings_seed.to_le_bytes();
    Pubkey::find_program_address(
        &[SEED_PREFIX, SEED_SETTINGS, seed_bytes.as_ref()],
        &PROGRAM_ID,
    )
}

pub fn derive_smart_account_pda(settings_pda: &Pubkey, account_index: u8) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            SEED_PREFIX,
            settings_pda.as_ref(),
            SEED_SMART_ACCOUNT,
            &[account_index],
        ],
        &PROGRAM_ID,
    )
}

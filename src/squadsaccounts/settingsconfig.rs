use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use crate::solanasetup::{PROGRAM_ID, SYSTEM_PROGRAM_ID, anchor_discriminator};

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct Permissions {
    pub mask: u8,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct SmartAccountSigner {
    pub key: Pubkey,
    pub permissions: Permissions,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct CreateSmartAccountArgs {
    pub settings_authority: Option<Pubkey>,
    pub threshold: u16,
    pub signers: Vec<SmartAccountSigner>,
    pub time_lock: u32,
    pub rent_collector: Option<Pubkey>,
    pub memo: Option<String>,
}

#[derive(Debug, BorshSerialize)]
pub struct ChangeThresholdArgs {
    pub new_threshold: u16,
    pub memo: Option<String>,
}

#[derive(Debug, BorshSerialize)]
pub struct AddSignerArgs {
    pub new_signer: SmartAccountSigner,
    pub memo: Option<String>,
}

#[derive(Debug, BorshSerialize)]
pub struct RemoveSignerArgs {
    pub old_signer: Pubkey,
    pub memo: Option<String>,
}

pub fn build_create_smart_account_instruction(
    program_config: Pubkey,
    treasury: Pubkey,
    creator: Pubkey,
    settings_pda: Pubkey,
    args: CreateSmartAccountArgs,
) -> Result<Instruction, String> {
    let discriminator = anchor_discriminator("global:create_smart_account");

    let encoded_args = borsh::to_vec(&args)
        .map_err(|error| format!("Failed to serialize create smart account arguments: {error}"))?;

    let mut data = Vec::with_capacity(discriminator.len() + encoded_args.len());

    data.extend_from_slice(&discriminator);
    data.extend_from_slice(&encoded_args);

    let accounts = vec![
        AccountMeta::new(program_config, false),
        AccountMeta::new(treasury, false),
        AccountMeta::new(creator, true),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        AccountMeta::new_readonly(PROGRAM_ID, false),
        AccountMeta::new(settings_pda, false),
    ];

    Ok(Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data,
    })
}

fn build_authority_settings_instruction<T: BorshSerialize>(
    instruction_name: &str,
    settings_pda: Pubkey,
    settings_authority: Pubkey,
    args: &T,
) -> Result<Instruction, String> {
    let discriminator = anchor_discriminator(instruction_name);
    let encoded_args = borsh::to_vec(args)
        .map_err(|error| format!("Failed to serialize settings arguments: {error}"))?;

    let mut data = Vec::with_capacity(discriminator.len() + encoded_args.len());
    data.extend_from_slice(&discriminator);
    data.extend_from_slice(&encoded_args);

    Ok(Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(settings_pda, false),
            AccountMeta::new_readonly(settings_authority, true),
            AccountMeta::new(settings_authority, true),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new_readonly(PROGRAM_ID, false),
        ],
        data,
    })
}

pub fn build_change_threshold_instruction(
    settings_pda: Pubkey,
    settings_authority: Pubkey,
    new_threshold: u16,
    memo: Option<String>,
) -> Result<Instruction, String> {
    build_authority_settings_instruction(
        "global:change_threshold_as_authority",
        settings_pda,
        settings_authority,
        &ChangeThresholdArgs {
            new_threshold,
            memo,
        },
    )
}

pub fn build_add_signer_instruction(
    settings_pda: Pubkey,
    settings_authority: Pubkey,
    signer: Pubkey,
    permissions_mask: u8,
    memo: Option<String>,
) -> Result<Instruction, String> {
    build_authority_settings_instruction(
        "global:add_signer_as_authority",
        settings_pda,
        settings_authority,
        &AddSignerArgs {
            new_signer: SmartAccountSigner {
                key: signer,
                permissions: Permissions {
                    mask: permissions_mask,
                },
            },
            memo,
        },
    )
}

pub fn build_remove_signer_instruction(
    settings_pda: Pubkey,
    settings_authority: Pubkey,
    signer: Pubkey,
    memo: Option<String>,
) -> Result<Instruction, String> {
    build_authority_settings_instruction(
        "global:remove_signer_as_authority",
        settings_pda,
        settings_authority,
        &RemoveSignerArgs {
            old_signer: signer,
            memo,
        },
    )
}

#[derive(Debug, Clone, BorshDeserialize)]
pub struct SettingsAccount {
    pub seed: u128,
    pub settings_authority: Pubkey,
    pub threshold: u16,
    pub time_lock: u32,
    pub transaction_index: u64,
    pub stale_transaction_index: u64,
    pub archival_authority: Option<Pubkey>,
    pub archivable_after: u64,
    pub bump: u8,
    pub signers: Vec<SmartAccountSigner>,
    pub account_utilization: u8,
    pub policy_seed: Option<u64>,
    pub reserved2: u8,
}

pub fn decode_settings_account(owner: &Pubkey, data: &[u8]) -> Result<SettingsAccount, String> {
    if owner != &PROGRAM_ID {
        return Err("Settings account has an unexpected owner".to_owned());
    }

    if data.len() < 8 {
        return Err("Settings account data is too short".to_owned());
    }

    let expected_discriminator = anchor_discriminator("account:Settings");

    if data[..8] != expected_discriminator {
        return Err("Invalid Settings account discriminator".to_owned());
    }

    SettingsAccount::try_from_slice(&data[8..])
        .map_err(|error| format!("Invalid Settings account data: {error}"))
}

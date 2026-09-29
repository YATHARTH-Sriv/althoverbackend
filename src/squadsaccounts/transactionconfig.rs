use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use crate::solanasetup::{
    PROGRAM_ID, SYSTEM_PROGRAM_ID, anchor_discriminator, derive_proposal_pda,
    derive_smart_account_pda, derive_transaction_pda,
};

#[derive(Debug, Clone, BorshSerialize)]
pub struct TransactionPayloadArgs {
    pub account_index: u8,
    pub ephemeral_signers: u8,
    pub transaction_message: Vec<u8>,
    pub memo: Option<String>,
}

#[derive(Debug, Clone, BorshSerialize)]
pub enum CreateTransactionArgs {
    TransactionPayload(TransactionPayloadArgs),
}

#[derive(Debug, Clone, BorshSerialize)]
pub struct CreateProposalArgs {
    pub transaction_index: u64,
    pub draft: bool,
}

#[derive(Debug, Clone, BorshSerialize)]
pub struct VoteOnProposalArgs {
    pub memo: Option<String>,
}

#[derive(Debug, Clone, BorshDeserialize)]
pub enum ProposalStatus {
    Draft { timestamp: i64 },
    Active { timestamp: i64 },
    Rejected { timestamp: i64 },
    Approved { timestamp: i64 },
    Executing,
    Executed { timestamp: i64 },
    Cancelled { timestamp: i64 },
}

impl ProposalStatus {
    pub fn database_value(&self) -> &'static str {
        match self {
            Self::Draft { .. } => "DRAFT",
            Self::Active { .. } => "PROPOSAL_ACTIVE",
            Self::Rejected { .. } => "REJECTED",
            Self::Approved { .. } => "APPROVED",
            Self::Executing => "EXECUTING",
            Self::Executed { .. } => "EXECUTED",
            Self::Cancelled { .. } => "CANCELLED",
        }
    }
}

#[derive(Debug, Clone, BorshDeserialize)]
pub struct ProposalAccount {
    pub settings: Pubkey,
    pub transaction_index: u64,
    pub rent_collector: Pubkey,
    pub status: ProposalStatus,
    pub bump: u8,
    pub approved: Vec<Pubkey>,
    pub rejected: Vec<Pubkey>,
    pub cancelled: Vec<Pubkey>,
}

#[derive(Debug, Clone)]
pub struct StoredTransactionMessage {
    pub account_index: u8,
    pub num_signers: u8,
    pub num_writable_signers: u8,
    pub num_writable_non_signers: u8,
    pub account_keys: Vec<Pubkey>,
}

fn instruction_data<T: BorshSerialize>(name: &str, args: &T) -> Result<Vec<u8>, String> {
    let mut data = anchor_discriminator(name).to_vec();
    data.extend(
        borsh::to_vec(args)
            .map_err(|error| format!("Failed to serialize {name} arguments: {error}"))?,
    );
    Ok(data)
}

fn push_small_vec_u8<T>(
    buffer: &mut Vec<u8>,
    values: &[T],
    encode: impl Fn(&T, &mut Vec<u8>),
) -> Result<(), String> {
    let length =
        u8::try_from(values.len()).map_err(|_| "Too many values for Squads message".to_owned())?;
    buffer.push(length);
    for value in values {
        encode(value, buffer);
    }
    Ok(())
}

pub fn serialize_sol_transfer_message(
    smart_account: Pubkey,
    recipient: Pubkey,
    amount_lamports: u64,
) -> Result<Vec<u8>, String> {
    let transfer =
        solana_system_interface::instruction::transfer(&smart_account, &recipient, amount_lamports);
    let account_keys = [smart_account, recipient, SYSTEM_PROGRAM_ID];
    let mut output = vec![1, 1, 1];
    push_small_vec_u8(&mut output, &account_keys, |key, bytes| {
        bytes.extend_from_slice(key.as_ref())
    })?;
    output.push(1);
    output.push(2);
    output.push(2);
    output.extend_from_slice(&[0, 1]);
    let data_len =
        u16::try_from(transfer.data.len()).map_err(|_| "Transfer data is too large".to_owned())?;
    output.extend_from_slice(&data_len.to_le_bytes());
    output.extend_from_slice(&transfer.data);
    output.push(0);
    Ok(output)
}

pub fn build_create_transaction_instruction(
    settings: Pubkey,
    creator: Pubkey,
    transaction_index: u64,
    account_index: u8,
    recipient: Pubkey,
    amount_lamports: u64,
    memo: Option<String>,
) -> Result<(Pubkey, Pubkey, Instruction), String> {
    let (transaction, _) = derive_transaction_pda(&settings, transaction_index);
    let (smart_account, _) = derive_smart_account_pda(&settings, account_index);
    let transaction_message =
        serialize_sol_transfer_message(smart_account, recipient, amount_lamports)?;
    let args = CreateTransactionArgs::TransactionPayload(TransactionPayloadArgs {
        account_index,
        ephemeral_signers: 0,
        transaction_message,
        memo,
    });
    Ok((
        transaction,
        smart_account,
        Instruction {
            program_id: PROGRAM_ID,
            accounts: vec![
                AccountMeta::new(settings, false),
                AccountMeta::new(transaction, false),
                AccountMeta::new_readonly(creator, true),
                AccountMeta::new(creator, true),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
                AccountMeta::new_readonly(PROGRAM_ID, false),
            ],
            data: instruction_data("global:create_transaction", &args)?,
        },
    ))
}

pub fn build_create_proposal_instruction(
    settings: Pubkey,
    creator: Pubkey,
    transaction_index: u64,
    draft: bool,
) -> Result<(Pubkey, Instruction), String> {
    let (proposal, _) = derive_proposal_pda(&settings, transaction_index);
    Ok((
        proposal,
        Instruction {
            program_id: PROGRAM_ID,
            accounts: vec![
                AccountMeta::new_readonly(settings, false),
                AccountMeta::new(proposal, false),
                AccountMeta::new_readonly(creator, true),
                AccountMeta::new(creator, true),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
                AccountMeta::new_readonly(PROGRAM_ID, false),
            ],
            data: instruction_data(
                "global:create_proposal",
                &CreateProposalArgs {
                    transaction_index,
                    draft,
                },
            )?,
        },
    ))
}

pub fn build_approve_proposal_instruction(
    settings: Pubkey,
    signer: Pubkey,
    transaction_index: u64,
) -> Result<(Pubkey, Instruction), String> {
    let (proposal, _) = derive_proposal_pda(&settings, transaction_index);
    Ok((
        proposal,
        Instruction {
            program_id: PROGRAM_ID,
            accounts: vec![
                AccountMeta::new_readonly(settings, false),
                AccountMeta::new(signer, true),
                AccountMeta::new(proposal, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
                AccountMeta::new_readonly(PROGRAM_ID, false),
            ],
            data: instruction_data(
                "global:approve_proposal",
                &VoteOnProposalArgs { memo: None },
            )?,
        },
    ))
}

pub fn build_execute_transaction_instruction(
    settings: Pubkey,
    signer: Pubkey,
    transaction_index: u64,
    message: &StoredTransactionMessage,
) -> Result<(Pubkey, Pubkey, Instruction), String> {
    let (proposal, _) = derive_proposal_pda(&settings, transaction_index);
    let (transaction, _) = derive_transaction_pda(&settings, transaction_index);
    let (smart_account, _) = derive_smart_account_pda(&settings, message.account_index);
    let mut accounts = vec![
        AccountMeta::new(settings, false),
        AccountMeta::new(proposal, false),
        AccountMeta::new_readonly(transaction, false),
        AccountMeta::new_readonly(signer, true),
        AccountMeta::new_readonly(PROGRAM_ID, false),
    ];
    for (index, key) in message.account_keys.iter().enumerate() {
        let is_signer = index < usize::from(message.num_signers) && *key != smart_account;
        let is_writable = index < usize::from(message.num_writable_signers)
            || (index >= usize::from(message.num_signers)
                && index
                    < usize::from(message.num_signers)
                        + usize::from(message.num_writable_non_signers));
        accounts.push(if is_writable {
            AccountMeta::new(*key, is_signer)
        } else {
            AccountMeta::new_readonly(*key, is_signer)
        });
    }
    Ok((
        proposal,
        transaction,
        Instruction {
            program_id: PROGRAM_ID,
            accounts,
            data: anchor_discriminator("global:execute_transaction").to_vec(),
        },
    ))
}

pub fn decode_proposal_account(owner: &Pubkey, data: &[u8]) -> Result<ProposalAccount, String> {
    if owner != &PROGRAM_ID
        || data.len() < 8
        || data[..8] != anchor_discriminator("account:Proposal")
    {
        return Err("Invalid Proposal account".to_owned());
    }
    ProposalAccount::deserialize(&mut &data[8..])
        .map_err(|error| format!("Invalid Proposal data: {error}"))
}

fn take<'a>(input: &mut &'a [u8], length: usize) -> Result<&'a [u8], String> {
    if input.len() < length {
        return Err("Transaction account data is truncated".to_owned());
    }
    let (value, rest) = input.split_at(length);
    *input = rest;
    Ok(value)
}

fn read_u32(input: &mut &[u8]) -> Result<u32, String> {
    let bytes: [u8; 4] = take(input, 4)?
        .try_into()
        .map_err(|_| "Transaction account data is truncated".to_owned())?;
    Ok(u32::from_le_bytes(bytes))
}

pub fn decode_transaction_message(
    owner: &Pubkey,
    data: &[u8],
) -> Result<StoredTransactionMessage, String> {
    if owner != &PROGRAM_ID
        || data.len() < 8
        || data[..8] != anchor_discriminator("account:Transaction")
    {
        return Err("Invalid Transaction account".to_owned());
    }
    let mut input = &data[8 + 32 + 32 + 32 + 8..];
    if take(&mut input, 1)?[0] != 0 {
        return Err("Transaction does not contain a standard payload".to_owned());
    }
    let account_index = take(&mut input, 1)?[0];
    let bump_len = usize::try_from(read_u32(&mut input)?)
        .map_err(|_| "Ephemeral signer list is too large".to_owned())?;
    if bump_len > 255 {
        return Err("Ephemeral signer list is too large".to_owned());
    }
    take(&mut input, bump_len)?;
    let num_signers = take(&mut input, 1)?[0];
    let num_writable_signers = take(&mut input, 1)?[0];
    let num_writable_non_signers = take(&mut input, 1)?[0];
    // The create instruction accepts a compact SmallVec with a u8 length, but
    // Squads stores the validated message on-chain as Vec<Pubkey> (u32 length).
    let key_count = usize::try_from(read_u32(&mut input)?)
        .map_err(|_| "Transaction account key list is too large".to_owned())?;
    if key_count > 256 {
        return Err("Transaction account key list is too large".to_owned());
    }
    let mut account_keys = Vec::with_capacity(key_count);
    for _ in 0..key_count {
        account_keys.push(Pubkey::new_from_array(
            take(&mut input, 32)?.try_into().unwrap(),
        ));
    }
    Ok(StoredTransactionMessage {
        account_index,
        num_signers,
        num_writable_signers,
        num_writable_non_signers,
        account_keys,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_message_uses_u32_vec_length_and_vault_is_not_outer_signer() {
        let settings = Pubkey::new_unique();
        let wallet = Pubkey::new_unique();
        let recipient = Pubkey::new_unique();
        let (smart_account, _) = derive_smart_account_pda(&settings, 0);

        let mut account_data = anchor_discriminator("account:Transaction").to_vec();
        account_data.extend_from_slice(settings.as_ref());
        account_data.extend_from_slice(wallet.as_ref());
        account_data.extend_from_slice(wallet.as_ref());
        account_data.extend_from_slice(&1_u64.to_le_bytes());
        account_data.push(0); // Payload::TransactionPayload
        account_data.push(0); // account_index
        account_data.extend_from_slice(&0_u32.to_le_bytes()); // ephemeral signer bumps
        account_data.extend_from_slice(&[1, 1, 1]);
        account_data.extend_from_slice(&3_u32.to_le_bytes());
        account_data.extend_from_slice(smart_account.as_ref());
        account_data.extend_from_slice(recipient.as_ref());
        account_data.extend_from_slice(SYSTEM_PROGRAM_ID.as_ref());

        let message = decode_transaction_message(&PROGRAM_ID, &account_data).unwrap();
        assert_eq!(message.account_keys[0], smart_account);

        let (_, _, instruction) =
            build_execute_transaction_instruction(settings, wallet, 1, &message).unwrap();

        let wallet_meta = instruction
            .accounts
            .iter()
            .find(|meta| meta.pubkey == wallet)
            .unwrap();
        let smart_account_meta = instruction
            .accounts
            .iter()
            .find(|meta| meta.pubkey == smart_account)
            .unwrap();

        assert!(wallet_meta.is_signer);
        assert!(!smart_account_meta.is_signer);
    }
}

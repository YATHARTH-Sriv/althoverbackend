use axum::{
    Json,
    extract::{Path, State},
};
use solana_sdk::pubkey::Pubkey;

use crate::{
    ApiError, AppState, bad_request, internal_error,
    squadsaccounts::{
        build_add_signer_instruction, build_change_threshold_instruction,
        build_remove_signer_instruction,
    },
    validation::parse_pubkey,
};

use super::models::*;
use super::{repository, service};

const VOTE_PERMISSION: u8 = 2;
const ALL_PERMISSIONS: u8 = 7;
fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

fn validate_permissions(mask: u8) -> Result<(), ApiError> {
    if mask == 0 || mask & !ALL_PERMISSIONS != 0 {
        return Err(bad_request(
            "permissionsMask must use Initiate (1), Approve (2), Execute (4), or a valid combination",
        ));
    }

    Ok(())
}

pub(crate) async fn refresh_settings_by_pda(
    state: &AppState,
    settings_pda: &Pubkey,
) -> Result<(), ApiError> {
    let indexed = repository::load_indexed_settings(state, settings_pda).await?;
    let chain = service::fetch_chain_settings(state, settings_pda).await?;
    repository::sync_settings_from_chain(state, &indexed, &chain).await
}

pub async fn build_change_threshold(
    State(state): State<AppState>,
    Json(payload): Json<ChangeThresholdBuildRequest>,
) -> Result<Json<BuildSettingsTransactionResponse>, ApiError> {
    let authority = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;

    repository::load_indexed_settings(&state, &settings_pda).await?;
    let chain = service::fetch_chain_settings(&state, &settings_pda).await?;
    service::ensure_authority(&chain, &authority)?;

    let vote_signer_count = chain
        .signers
        .iter()
        .filter(|signer| signer.permissions.mask & VOTE_PERMISSION != 0)
        .count();

    if payload.new_threshold == 0 || usize::from(payload.new_threshold) > vote_signer_count {
        return Err(bad_request(format!(
            "Threshold must be between 1 and {vote_signer_count} approve-capable signer{}",
            if vote_signer_count == 1 { "" } else { "s" }
        )));
    }

    let memo = normalize_optional_string(payload.memo)
        .unwrap_or_else(|| "Hover Agent threshold update".to_owned());
    let instruction = build_change_threshold_instruction(
        settings_pda,
        authority,
        payload.new_threshold,
        Some(memo.clone()),
    )
    .map_err(internal_error)?;
    let transaction_base64 =
        service::build_unsigned_settings_transaction(&state, authority, instruction).await?;

    Ok(Json(BuildSettingsTransactionResponse {
        wallet_address: authority.to_string(),
        settings_pda: settings_pda.to_string(),
        signer: None,
        permissions_mask: None,
        new_threshold: Some(payload.new_threshold),
        memo,
        transaction_base64,
    }))
}

pub async fn change_threshold_submitted(
    State(state): State<AppState>,
    Json(payload): Json<ChangeThresholdSubmittedRequest>,
) -> Result<Json<SubmittedSettingsResponse>, ApiError> {
    let authority = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let indexed = repository::load_indexed_settings(&state, &settings_pda).await?;
    let signature =
        service::verify_submitted_transaction(&state, &payload.tx_sig, &authority, &settings_pda)
            .await?;
    let chain = service::fetch_chain_settings(&state, &settings_pda).await?;

    service::ensure_authority(&chain, &authority)?;
    if chain.threshold != payload.new_threshold {
        return Err(bad_request(
            "Confirmed transaction did not apply the requested threshold",
        ));
    }

    repository::sync_settings_from_chain(&state, &indexed, &chain).await?;
    repository::log_activity(
        &state,
        &indexed,
        "THRESHOLD_CHANGED",
        "Threshold changed",
        &signature,
        serde_json::json!({ "newThreshold": payload.new_threshold }),
    )
    .await?;

    Ok(Json(SubmittedSettingsResponse {
        settings: repository::fetch_settings_response(&state, indexed.id).await?,
    }))
}

pub async fn build_add_signer(
    State(state): State<AppState>,
    Json(payload): Json<AddSignerBuildRequest>,
) -> Result<Json<BuildSettingsTransactionResponse>, ApiError> {
    let authority = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let signer = parse_pubkey(&payload.signer, "signer")?;
    let permissions_mask = payload.permissions_mask.unwrap_or(ALL_PERMISSIONS);

    validate_permissions(permissions_mask)?;
    repository::load_indexed_settings(&state, &settings_pda).await?;
    let chain = service::fetch_chain_settings(&state, &settings_pda).await?;
    service::ensure_authority(&chain, &authority)?;

    if chain.signers.iter().any(|existing| existing.key == signer) {
        return Err(bad_request(
            "This wallet is already a signer on the settings account",
        ));
    }

    let memo = normalize_optional_string(payload.memo)
        .unwrap_or_else(|| "Hover Agent signer update addition".to_owned());
    let instruction = build_add_signer_instruction(
        settings_pda,
        authority,
        signer,
        permissions_mask,
        Some(memo.clone()),
    )
    .map_err(internal_error)?;
    let transaction_base64 =
        service::build_unsigned_settings_transaction(&state, authority, instruction).await?;

    Ok(Json(BuildSettingsTransactionResponse {
        wallet_address: authority.to_string(),
        settings_pda: settings_pda.to_string(),
        signer: Some(signer.to_string()),
        permissions_mask: Some(permissions_mask),
        new_threshold: None,
        memo,
        transaction_base64,
    }))
}

pub async fn add_signer_submitted(
    State(state): State<AppState>,
    Json(payload): Json<AddSignerSubmittedRequest>,
) -> Result<Json<AddSignerSubmittedResponse>, ApiError> {
    let authority = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let signer = parse_pubkey(&payload.signer, "signer")?;
    let permissions_mask = payload.permissions_mask.unwrap_or(ALL_PERMISSIONS);

    validate_permissions(permissions_mask)?;
    let indexed = repository::load_indexed_settings(&state, &settings_pda).await?;
    let signature =
        service::verify_submitted_transaction(&state, &payload.tx_sig, &authority, &settings_pda)
            .await?;
    let chain = service::fetch_chain_settings(&state, &settings_pda).await?;
    service::ensure_authority(&chain, &authority)?;

    let confirmed_signer = chain
        .signers
        .iter()
        .find(|existing| existing.key == signer)
        .ok_or_else(|| bad_request("Confirmed transaction did not add the requested signer"))?;

    if confirmed_signer.permissions.mask != permissions_mask {
        return Err(bad_request(
            "Confirmed transaction did not apply the requested signer permissions",
        ));
    }

    repository::sync_settings_from_chain(&state, &indexed, &chain).await?;

    let name = normalize_optional_string(payload.name);
    let email = normalize_optional_string(payload.email);
    let designation = normalize_optional_string(payload.designation);
    let invite = repository::upsert_signer_profile_and_invite(
        &state,
        repository::SignerInviteInput {
            indexed: &indexed,
            authority: &authority,
            signer: &signer,
            permissions_mask,
            name: name.clone(),
            email: email.clone(),
            designation: designation.clone(),
        },
    )
    .await?;

    repository::log_activity(
        &state,
        &indexed,
        "SIGNER_ADDED",
        "Signer added",
        &signature,
        serde_json::json!({
            "signer": signer.to_string(),
            "permissionsMask": permissions_mask,
            "name": name,
            "email": email,
            "designation": designation
        }),
    )
    .await?;

    Ok(Json(AddSignerSubmittedResponse {
        settings: repository::fetch_settings_response(&state, indexed.id).await?,
        invite,
    }))
}

pub async fn build_remove_signer(
    State(state): State<AppState>,
    Json(payload): Json<RemoveSignerBuildRequest>,
) -> Result<Json<BuildSettingsTransactionResponse>, ApiError> {
    let authority = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let signer = parse_pubkey(&payload.signer, "signer")?;

    repository::load_indexed_settings(&state, &settings_pda).await?;
    let chain = service::fetch_chain_settings(&state, &settings_pda).await?;
    service::ensure_authority(&chain, &authority)?;

    if !chain.signers.iter().any(|existing| existing.key == signer) {
        return Err(bad_request(
            "This wallet is not currently a signer on the settings account",
        ));
    }
    if chain.signers.len() <= 1 {
        return Err(bad_request("Cannot remove the last signer"));
    }

    let remaining_vote_signers = chain
        .signers
        .iter()
        .filter(|existing| {
            existing.key != signer && existing.permissions.mask & VOTE_PERMISSION != 0
        })
        .count();

    if usize::from(chain.threshold) > remaining_vote_signers {
        return Err(bad_request(format!(
            "Lower the threshold before removing this signer. Current threshold is {}, but only {remaining_vote_signers} approve-capable signer{} would remain.",
            chain.threshold,
            if remaining_vote_signers == 1 { "" } else { "s" }
        )));
    }

    let memo = normalize_optional_string(payload.memo)
        .unwrap_or_else(|| format!("Hover Agent signer update removing {signer}"));
    let instruction =
        build_remove_signer_instruction(settings_pda, authority, signer, Some(memo.clone()))
            .map_err(internal_error)?;
    let transaction_base64 =
        service::build_unsigned_settings_transaction(&state, authority, instruction).await?;

    Ok(Json(BuildSettingsTransactionResponse {
        wallet_address: authority.to_string(),
        settings_pda: settings_pda.to_string(),
        signer: Some(signer.to_string()),
        permissions_mask: None,
        new_threshold: None,
        memo,
        transaction_base64,
    }))
}

pub async fn remove_signer_submitted(
    State(state): State<AppState>,
    Json(payload): Json<RemoveSignerSubmittedRequest>,
) -> Result<Json<SubmittedSettingsResponse>, ApiError> {
    let authority = parse_pubkey(&payload.wallet_address, "walletAddress")?;
    let settings_pda = parse_pubkey(&payload.settings_pda, "settingsPda")?;
    let signer = parse_pubkey(&payload.signer, "signer")?;
    let indexed = repository::load_indexed_settings(&state, &settings_pda).await?;
    let signature =
        service::verify_submitted_transaction(&state, &payload.tx_sig, &authority, &settings_pda)
            .await?;
    let chain = service::fetch_chain_settings(&state, &settings_pda).await?;

    service::ensure_authority(&chain, &authority)?;
    if chain.signers.iter().any(|existing| existing.key == signer) {
        return Err(bad_request(
            "Confirmed transaction did not remove the requested signer",
        ));
    }

    repository::sync_settings_from_chain(&state, &indexed, &chain).await?;

    repository::revoke_pending_invites(&state, indexed.id, &signer).await?;

    repository::log_activity(
        &state,
        &indexed,
        "SIGNER_REMOVED",
        "Signer removed",
        &signature,
        serde_json::json!({ "signer": signer.to_string() }),
    )
    .await?;

    Ok(Json(SubmittedSettingsResponse {
        settings: repository::fetch_settings_response(&state, indexed.id).await?,
    }))
}

pub async fn refresh_settings(
    State(state): State<AppState>,
    Path(id_or_pda): Path<String>,
) -> Result<Json<SubmittedSettingsResponse>, ApiError> {
    let indexed = repository::find_indexed_settings(&state, &id_or_pda).await?;

    let settings_pda = parse_pubkey(&indexed.pda, "settingsPda")?;
    let chain = service::fetch_chain_settings(&state, &settings_pda).await?;
    repository::sync_settings_from_chain(&state, &indexed, &chain).await?;

    Ok(Json(SubmittedSettingsResponse {
        settings: repository::fetch_settings_response(&state, indexed.id).await?,
    }))
}

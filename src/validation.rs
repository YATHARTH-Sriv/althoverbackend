use std::str::FromStr;

use solana_sdk::{pubkey::Pubkey, signature::Signature};

use crate::{ApiError, bad_request};

pub fn required_text(value: &str, field: &str, max: usize) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(bad_request(format!("{field} is required")));
    }
    if value.chars().count() > max {
        return Err(bad_request(format!("{field} is too long")));
    }
    Ok(value.to_owned())
}

pub fn optional_text(
    value: Option<String>,
    field: &str,
    max: usize,
) -> Result<Option<String>, ApiError> {
    value
        .map(|value| {
            let value = value.trim();
            if value.is_empty() {
                Ok(None)
            } else if value.chars().count() > max {
                Err(bad_request(format!("{field} is too long")))
            } else {
                Ok(Some(value.to_owned()))
            }
        })
        .transpose()
        .map(Option::flatten)
}

pub fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

pub fn parse_pubkey(value: &str, field: &str) -> Result<Pubkey, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(bad_request(format!("{field} is required")));
    }
    Pubkey::from_str(value).map_err(|_| bad_request(format!("{field} is invalid")))
}

pub fn parse_signature(value: &str, field: &str) -> Result<Signature, ApiError> {
    Signature::from_str(value.trim()).map_err(|_| bad_request(format!("{field} is invalid")))
}

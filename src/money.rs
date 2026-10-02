use crate::{ApiError, bad_request};

pub fn parse_decimal_units(value: &str, scale: u32, field: &str) -> Result<i128, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('-') || value.starts_with('+') {
        return Err(bad_request(format!("{field} must be a positive decimal")));
    }
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > scale as usize
    {
        return Err(bad_request(format!(
            "{field} must be a decimal with at most {scale} decimal places"
        )));
    }
    let factor = 10_i128.pow(scale);
    let whole = whole
        .parse::<i128>()
        .map_err(|_| bad_request(format!("{field} is too large")))?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<i128>()
            .map_err(|_| bad_request(format!("{field} is invalid")))?
            * 10_i128.pow(scale - fraction.len() as u32)
    };
    whole
        .checked_mul(factor)
        .and_then(|whole| whole.checked_add(fraction))
        .ok_or_else(|| bad_request(format!("{field} is too large")))
}

pub fn format_units(units: i128, scale: u32) -> String {
    let factor = 10_i128.pow(scale);
    format!(
        "{}.{:0width$}",
        units / factor,
        units % factor,
        width = scale as usize
    )
}

pub fn normalize_decimal(value: &str, scale: u32, field: &str) -> Result<String, ApiError> {
    parse_decimal_units(value, scale, field).map(|units| format_units(units, scale))
}

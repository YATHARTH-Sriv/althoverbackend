pub const DEFAULT_PAGE_SIZE: i64 = 50;
pub const MAX_PAGE_SIZE: i64 = 100;

pub fn bounds(limit: Option<i64>, offset: Option<i64>) -> (i64, i64) {
    (
        limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE),
        offset.unwrap_or(0).max(0),
    )
}

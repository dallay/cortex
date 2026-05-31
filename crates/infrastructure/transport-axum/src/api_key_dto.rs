use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginationDto {
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListApiKeysResponseDto<T> {
    pub keys: Vec<T>,
    pub pagination: PaginationDto,
}

impl<T> ListApiKeysResponseDto<T> {
    pub fn new(keys: Vec<T>, total: i64, limit: i64, offset: i64) -> Self {
        Self {
            keys,
            pagination: PaginationDto {
                total,
                limit,
                offset,
            },
        }
    }
}

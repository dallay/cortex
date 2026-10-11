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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_api_keys_response_dto_serialization() {
        let dto = ListApiKeysResponseDto::new(vec!["key_1", "key_2"], 2, 10, 0);
        let val = serde_json::to_value(&dto).unwrap();
        assert_eq!(val["keys"], serde_json::json!(["key_1", "key_2"]));
        assert_eq!(
            val["pagination"],
            serde_json::json!({
                "total": 2,
                "limit": 10,
                "offset": 0
            })
        );
    }
}

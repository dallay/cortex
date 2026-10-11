use transport_axum::api_key_dto::ListApiKeysResponseDto;

#[test]
fn test_list_api_keys_response_dto_serialization() {
    let dto = ListApiKeysResponseDto::new(vec!["key_1", "key_2"], 2, 10, 0);
    let json = serde_json::to_string(&dto).unwrap();
    assert!(json.contains("\"total\":2"));
    assert!(json.contains("\"limit\":10"));
    assert!(json.contains("\"offset\":0"));
    assert!(json.contains("\"keys\":[\"key_1\",\"key_2\"]"));
}

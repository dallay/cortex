use muninn_neo4j::bolt::{from_json, opt_string, opt_vector, to_datetime, to_json, Props};
use neo4rs::{BoltMap, BoltType};
use uuid::Uuid;

#[test]
fn props_rejects_non_map_or_node() {
    let invalid = BoltType::Integer(neo4rs::BoltInteger { value: 42 });
    assert!(Props::of(&invalid).is_err());
}

#[test]
fn props_reads_and_validates_basic_fields() {
    let mut map = BoltMap::new();
    let u = Uuid::new_v4();
    map.put("id".into(), BoltType::from(u.to_string().as_str()));
    map.put("bad_uuid".into(), BoltType::from("not-a-uuid"));
    map.put("title".into(), BoltType::from("Test Title"));
    map.put("active".into(), BoltType::from(true));
    map.put("count".into(), BoltType::from(42i64));
    map.put("score_int".into(), BoltType::from(100i64));
    map.put(
        "score_float".into(),
        BoltType::Float(neo4rs::BoltFloat { value: 99.5 }),
    );
    map.put("empty_str".into(), BoltType::from(""));
    map.put("null_field".into(), BoltType::Null(neo4rs::BoltNull));

    let bolt_map = BoltType::Map(map);
    let props = Props::of(&bolt_map).expect("valid map props");

    // String and UUID
    assert_eq!(props.string("title").as_deref(), Some("Test Title"));
    assert_eq!(props.required_string("title").unwrap(), "Test Title");
    assert!(props.required_string("missing").is_err());
    assert_eq!(props.uuid("id").unwrap(), u);
    assert_eq!(props.opt_uuid("id"), Some(u));
    assert!(props.uuid("bad_uuid").is_err());
    assert_eq!(props.opt_uuid("bad_uuid"), None);
    assert_eq!(props.opt_uuid("missing"), None);

    // Booleans and numbers
    assert_eq!(props.bool("active"), Some(true));
    assert_eq!(props.bool("missing"), None);
    assert_eq!(props.i64("count"), Some(42));
    assert_eq!(props.i64("missing"), None);
    assert_eq!(props.f64("score_int"), Some(100.0));
    assert_eq!(props.f64("score_float"), Some(99.5));
    assert_eq!(props.f64("missing"), None);

    // Null handling
    assert!(props.raw("null_field").is_none());
}

#[test]
fn props_reads_lists() {
    let mut map = BoltMap::new();
    map.put(
        "tags".into(),
        BoltType::List(vec![BoltType::from("tag1"), BoltType::from("tag2")].into()),
    );
    map.put(
        "embeddings".into(),
        BoltType::List(
            vec![
                BoltType::Float(neo4rs::BoltFloat { value: 1.5 }),
                BoltType::from(2i64),
            ]
            .into(),
        ),
    );

    let bolt_map = BoltType::Map(map);
    let props = Props::of(&bolt_map).unwrap();

    assert_eq!(props.string_list("tags"), vec!["tag1", "tag2"]);
    assert_eq!(props.string_list("missing"), Vec::<String>::new());
    assert_eq!(props.f64_list("embeddings"), Some(vec![1.5, 2.0]));
    assert_eq!(props.f64_list("missing"), None);
}

#[test]
fn to_datetime_parses_rfc3339_and_invalid() {
    let valid_rfc = BoltType::from("2026-11-18T10:00:00Z");
    assert!(to_datetime(&valid_rfc).is_some());

    let invalid_str = BoltType::from("not-a-datetime");
    assert!(to_datetime(&invalid_str).is_none());

    let non_date = BoltType::from(12345i64);
    assert!(to_datetime(&non_date).is_none());
}

#[test]
fn to_json_and_from_json_round_trips() {
    // Null
    let null_val = serde_json::Value::Null;
    let null_bolt = from_json(&null_val);
    assert_eq!(to_json(&null_bolt), serde_json::Value::Null);

    // Boolean
    let bool_val = serde_json::json!(true);
    let bool_bolt = from_json(&bool_val);
    assert_eq!(to_json(&bool_bolt), bool_val);

    // Integer
    let int_val = serde_json::json!(42);
    let int_bolt = from_json(&int_val);
    assert_eq!(to_json(&int_bolt), int_val);

    // Float
    let float_val = serde_json::json!(12.34);
    let float_bolt = from_json(&float_val);
    assert_eq!(to_json(&float_bolt), float_val);

    // String
    let str_val = serde_json::json!("hello world");
    let str_bolt = from_json(&str_val);
    assert_eq!(to_json(&str_bolt), str_val);

    // Array
    let arr_val = serde_json::json!([1, "a", true]);
    let arr_bolt = from_json(&arr_val);
    assert_eq!(to_json(&arr_bolt), arr_val);

    // Object
    let obj_val = serde_json::json!({"key": "value", "count": 10});
    let obj_bolt = from_json(&obj_val);
    assert_eq!(to_json(&obj_bolt), obj_val);
}

#[test]
fn opt_helpers_construct_expected_bolt_types() {
    // opt_string
    let some_s = opt_string(Some("test"));
    assert!(matches!(some_s, BoltType::String(_)));
    let none_s = opt_string(None);
    assert!(matches!(none_s, BoltType::Null(_)));

    // opt_vector
    let some_v = opt_vector(Some(&[1.0, 2.0, 3.0]));
    assert!(matches!(some_v, BoltType::List(_)));
    let none_v = opt_vector(None);
    assert!(matches!(none_v, BoltType::Null(_)));
}

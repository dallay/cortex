//! Conversions between Bolt values, JSON and domain primitives.

use chrono::{DateTime, FixedOffset, NaiveDateTime, Utc};
use muninn_core::{MemoryError, MemoryResult};
use neo4rs::{BoltMap, BoltType, Row};
use serde_json::{Map, Number, Value};
use uuid::Uuid;

/// Read-only view over a node's (or map's) properties.
pub struct Props<'a>(&'a BoltMap);

impl<'a> Props<'a> {
    pub fn of(value: &'a BoltType) -> MemoryResult<Self> {
        match value {
            BoltType::Node(node) => Ok(Self(&node.properties)),
            BoltType::Relation(rel) => Ok(Self(&rel.properties)),
            BoltType::Map(map) => Ok(Self(map)),
            _ => Err(MemoryError::storage("expected a node or map")),
        }
    }

    pub fn raw(&self, key: &str) -> Option<&'a BoltType> {
        match self.0.value.get(key) {
            None | Some(BoltType::Null(_)) => None,
            Some(value) => Some(value),
        }
    }

    pub fn string(&self, key: &str) -> Option<String> {
        match self.raw(key)? {
            BoltType::String(text) => Some(text.value.clone()),
            _ => None,
        }
    }

    pub fn required_string(&self, key: &str) -> MemoryResult<String> {
        self.string(key)
            .ok_or_else(|| MemoryError::storage(format!("missing string property '{key}'")))
    }

    pub fn uuid(&self, key: &str) -> MemoryResult<Uuid> {
        let text = self.required_string(key)?;
        Uuid::parse_str(&text)
            .map_err(|_| MemoryError::storage(format!("property '{key}' is not a UUID")))
    }

    pub fn opt_uuid(&self, key: &str) -> Option<Uuid> {
        self.string(key)
            .and_then(|text| Uuid::parse_str(&text).ok())
    }

    pub fn datetime(&self, key: &str) -> Option<DateTime<Utc>> {
        self.raw(key).and_then(to_datetime)
    }

    /// Upstream falls back to "now" when a timestamp is missing.
    pub fn datetime_or_now(&self, key: &str) -> DateTime<Utc> {
        self.datetime(key).unwrap_or_else(Utc::now)
    }

    pub fn f64(&self, key: &str) -> Option<f64> {
        match self.raw(key)? {
            BoltType::Float(value) => Some(value.value),
            #[allow(clippy::cast_precision_loss)]
            BoltType::Integer(value) => Some(value.value as f64),
            _ => None,
        }
    }

    pub fn i64(&self, key: &str) -> Option<i64> {
        match self.raw(key)? {
            BoltType::Integer(value) => Some(value.value),
            _ => None,
        }
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.raw(key)? {
            BoltType::Boolean(value) => Some(value.value),
            _ => None,
        }
    }

    pub fn f64_list(&self, key: &str) -> Option<Vec<f64>> {
        match self.raw(key)? {
            BoltType::List(list) => list
                .value
                .iter()
                .map(|item| match item {
                    BoltType::Float(value) => Some(value.value),
                    #[allow(clippy::cast_precision_loss)]
                    BoltType::Integer(value) => Some(value.value as f64),
                    _ => None,
                })
                .collect(),
            _ => None,
        }
    }

    pub fn string_list(&self, key: &str) -> Vec<String> {
        match self.raw(key) {
            Some(BoltType::List(list)) => list
                .value
                .iter()
                .filter_map(|item| match item {
                    BoltType::String(text) => Some(text.value.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

pub fn to_datetime(value: &BoltType) -> Option<DateTime<Utc>> {
    match value {
        BoltType::DateTime(value) => DateTime::<FixedOffset>::try_from(value)
            .ok()
            .map(|dt| dt.with_timezone(&Utc)),
        BoltType::DateTimeZoneId(value) => DateTime::<FixedOffset>::try_from(value)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|| NaiveDateTime::try_from(value).ok().map(|dt| dt.and_utc())),
        BoltType::LocalDateTime(value) => {
            NaiveDateTime::try_from(value).ok().map(|dt| dt.and_utc())
        }
        BoltType::String(text) => DateTime::parse_from_rfc3339(&text.value)
            .ok()
            .map(|dt| dt.with_timezone(&Utc)),
        _ => None,
    }
}

/// Fetch a column from a row as a raw Bolt value.
pub fn column(row: &Row, key: &str) -> MemoryResult<BoltType> {
    row.get::<BoltType>(key)
        .map_err(|err| MemoryError::storage(format!("column '{key}': {err}")))
}

/// Convert a Bolt value to JSON (temporal values become RFC 3339 strings,
/// nodes and relationships become their property maps).
pub fn to_json(value: &BoltType) -> Value {
    match value {
        BoltType::Null(_) => Value::Null,
        BoltType::String(text) => Value::String(text.value.clone()),
        BoltType::Boolean(value) => Value::Bool(value.value),
        BoltType::Integer(value) => Value::from(value.value),
        BoltType::Float(value) => Number::from_f64(value.value).map_or(Value::Null, Value::Number),
        BoltType::List(list) => Value::Array(list.value.iter().map(to_json).collect()),
        BoltType::Map(map) => map_to_json(map),
        BoltType::Node(node) => map_to_json(&node.properties),
        BoltType::Relation(rel) => map_to_json(&rel.properties),
        BoltType::UnboundedRelation(rel) => map_to_json(&rel.properties),
        BoltType::DateTime(_) | BoltType::DateTimeZoneId(_) | BoltType::LocalDateTime(_) => {
            to_datetime(value).map_or(Value::Null, |dt| Value::String(dt.to_rfc3339()))
        }
        BoltType::Point2D(point) => serde_json::json!({"x": point.x.value, "y": point.y.value}),
        BoltType::Point3D(point) => {
            serde_json::json!({"x": point.x.value, "y": point.y.value, "z": point.z.value})
        }
        _ => Value::String(format!("{value:?}")),
    }
}

fn map_to_json(map: &BoltMap) -> Value {
    let mut out = Map::new();
    for (key, item) in &map.value {
        out.insert(key.value.clone(), to_json(item));
    }
    Value::Object(out)
}

/// Convert JSON into a Bolt parameter.
pub fn from_json(value: &Value) -> BoltType {
    match value {
        Value::Null => BoltType::Null(neo4rs::BoltNull),
        Value::Bool(flag) => BoltType::from(*flag),
        Value::Number(number) => number.as_i64().map_or_else(
            || BoltType::from(number.as_f64().unwrap_or(f64::NAN)),
            BoltType::from,
        ),
        Value::String(text) => BoltType::from(text.as_str()),
        Value::Array(items) => {
            BoltType::List(items.iter().map(from_json).collect::<Vec<_>>().into())
        }
        Value::Object(map) => {
            let mut out = BoltMap::new();
            for (key, item) in map {
                out.put(key.as_str().into(), from_json(item));
            }
            BoltType::Map(out)
        }
    }
}

/// Collect selected columns of a row into one map value.
pub fn row_values(row: &Row, keys: &[&str]) -> MemoryResult<BoltType> {
    let mut map = BoltMap::new();
    for key in keys {
        map.put((*key).into(), column(row, key)?);
    }
    Ok(BoltType::Map(map))
}

pub fn opt_string(value: Option<&str>) -> BoltType {
    value.map_or(BoltType::Null(neo4rs::BoltNull), BoltType::from)
}

pub fn opt_vector(value: Option<&[f64]>) -> BoltType {
    value.map_or(BoltType::Null(neo4rs::BoltNull), |vector| {
        BoltType::from(vector.to_vec())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

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

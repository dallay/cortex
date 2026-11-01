//! Wire codecs that must match the upstream Python representation.
//!
//! Upstream stores metadata and tool payloads as `json.dumps(value)` strings,
//! so graph-level parity requires reproducing Python's default encoder:
//! `", "` / `": "` separators and `ensure_ascii=True` escaping.

use std::fmt::Write as _;

use serde_json::Value;

use crate::model::Metadata;

/// Serialize like Python's `json.dumps(value)` with default arguments.
pub fn python_json_dumps(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

/// `short_term._serialize_metadata`: empty metadata is stored as `null`.
pub fn serialize_metadata(metadata: &Metadata) -> Option<String> {
    if metadata.is_empty() {
        None
    } else {
        Some(python_json_dumps(&Value::Object(metadata.clone())))
    }
}

/// `short_term._deserialize_metadata`: invalid or missing JSON yields `{}`.
pub fn deserialize_metadata(raw: Option<&str>) -> Metadata {
    raw.and_then(|text| serde_json::from_str::<Value>(text).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => write_number(out, number),
        Value::String(text) => write_string(out, text),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (index, (key, item)) in map.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_string(out, key);
                out.push_str(": ");
                write_value(out, item);
            }
            out.push('}');
        }
    }
}

fn write_number(out: &mut String, number: &serde_json::Number) {
    if let Some(float) = number.as_f64().filter(|_| number.is_f64()) {
        // Python repr: integral floats keep a trailing `.0`.
        if float.is_finite() && float.fract() == 0.0 && float.abs() < 1e16 {
            let _ = write!(out, "{float:.1}");
        } else {
            let _ = write!(out, "{float}");
        }
    } else {
        let _ = write!(out, "{number}");
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ch if ch.is_ascii() && !ch.is_ascii_control() => out.push(ch),
            ch => {
                let mut units = [0u16; 2];
                for unit in ch.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn matches_python_default_separators_and_ascii_escaping() {
        let value = json!({"b": [1, 2.0, true, null], "a": "café 🎉\n"});
        // serde_json sorts keys while Python keeps insertion order; the spec
        // treats JSON key order as a representational difference (§10.2).
        assert_eq!(
            python_json_dumps(&value),
            concat!(
                r#"{"a": "caf\u"#,
                "00e9 ",
                r#"\u"#,
                "d83c",
                r#"\u"#,
                r#"df89\n", "b": [1, 2.0, true, null]}"#
            )
        );
    }

    #[test]
    fn empty_metadata_is_stored_as_null() {
        assert_eq!(serialize_metadata(&Metadata::new()), None);
        assert!(deserialize_metadata(Some("not json")).is_empty());
    }
}

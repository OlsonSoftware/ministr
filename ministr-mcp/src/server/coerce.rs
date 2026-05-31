//! Forgiving serde coercions for MCP tool arguments.
//!
//! Agents routinely send a single string where a tool expects a string array
//! (e.g. `principles: "srp"` instead of `["srp"]`), a numeric string where a
//! tool expects a number (`top_k: "10"`), or simply omit a "required" field.
//! The strict serde default rejects these with a `-32602 invalid type` /
//! `missing field` JSON-RPC error — and in Claude Code that error
//! cascade-cancels *every sibling tool call in the same parallel batch*
//! (anthropics/claude-code#22264), stalling the whole turn.
//!
//! This module is the single home (SRP) for the "never reject an argument"
//! discipline: every helper here returns a value or a sane fallback and is
//! **infallible by construction** — it never produces a `D::Error` for a
//! shape mismatch. Arg structs pair these with `#[serde(default)]` so an
//! absent field is also fine. The net guarantee: argument deserialization for
//! a ministr MCP tool cannot emit `-32602`, so it can never be the errored
//! sibling that triggers the cascade. Required-ness is enforced *inside the
//! handler* as a soft, non-error result (see [`super::helpers::soft_error`]),
//! not at this layer.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer};

/// Coerce a JSON value into a `u64` as forgivingly as is sane, never failing.
/// Accepts an integer, a float (truncated, floored at 0), or a numeric string.
/// Anything unparseable falls back to `0`, which handlers treat as "unset".
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "deliberately lossy: a forgiving coercion floors at 0 and truncates the fraction"
)]
fn coerce_u64(value: &serde_json::Value) -> u64 {
    use serde_json::Value;
    match value {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().map(|f| f.max(0.0) as u64))
            .unwrap_or(0),
        Value::String(s) => s.trim().parse::<u64>().unwrap_or(0),
        _ => 0,
    }
}

/// Coerce a JSON value into an `f64`, never failing. Accepts a number or a
/// numeric string; anything else (incl. non-finite) falls back to `0.0`.
fn coerce_f64(value: &serde_json::Value) -> f64 {
    use serde_json::Value;
    let raw = match value {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    };
    if raw.is_finite() { raw } else { 0.0 }
}

/// Normalize a JSON value into a `Vec<String>` as forgivingly as is sane.
fn coerce_string_vec(value: serde_json::Value) -> Vec<String> {
    use serde_json::Value;
    match value {
        Value::Null => Vec::new(),
        Value::String(s) => {
            if s.trim().is_empty() {
                Vec::new()
            } else {
                vec![s]
            }
        }
        Value::Array(items) => items
            .into_iter()
            .map(|item| match item {
                Value::String(s) => s,
                other => other.to_string(),
            })
            .collect(),
        other => vec![other.to_string()],
    }
}

/// Coerce a JSON value into a `String`. `null` and absent → `""`; a scalar is
/// stringified (without surrounding quotes); an object/array is rendered as
/// compact JSON. Never errors. Use for "required" string fields; the handler
/// validates the empty case and returns a soft error.
pub fn lenient_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    use serde_json::Value;
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        Value::Null => String::new(),
        Value::String(s) => s,
        other => other.to_string(),
    })
}

/// Like [`lenient_string`] but yields `Option<String>`: `null`/absent → `None`,
/// empty string → `None`, every other shape coerces to `Some(String)`.
pub fn lenient_opt_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde_json::Value;
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        Value::Null => None,
        Value::String(s) => {
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        other => Some(other.to_string()),
    })
}

/// `#[serde(deserialize_with = "coerce::string_or_seq")]` — accept a string
/// array, a single string (wrapped into a one-element list), `null` (empty),
/// or a scalar (stringified). Pair with `#[serde(default)]`.
pub fn string_or_seq<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(coerce_string_vec(value))
}

/// Like [`string_or_seq`] but yields `Option<Vec<String>>`: `null`/absent →
/// `None`, every other shape coerces to `Some(Vec<String>)`. Preserves the
/// distinction some handlers draw between "omitted" and "empty list".
pub fn opt_string_or_seq<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Null => None,
        other => Some(coerce_string_vec(other)),
    })
}

/// `#[serde(deserialize_with = "coerce::lenient_opt_usize")]` — accept an
/// integer, a numeric string, or anything else (→ `None`/`Some(0)`). Never
/// errors. `null`/absent → `None`; any other shape → `Some(usize)`.
pub fn lenient_opt_usize<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Null => None,
        other => Some(usize::try_from(coerce_u64(&other)).unwrap_or(usize::MAX)),
    })
}

/// `#[serde(deserialize_with = "coerce::lenient_opt_u32")]` — like
/// [`lenient_opt_usize`] but yields `Option<u32>`.
pub fn lenient_opt_u32<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Null => None,
        other => Some(u32::try_from(coerce_u64(&other)).unwrap_or(u32::MAX)),
    })
}

/// `#[serde(deserialize_with = "coerce::lenient_opt_f32")]` — accept a number
/// or numeric string (→ `Some(f32)`), `null`/absent → `None`. Non-finite and
/// unparseable values collapse to `Some(0.0)`. Never errors.
#[allow(
    clippy::cast_possible_truncation,
    reason = "f64→f32 narrowing is acceptable for a threshold knob"
)]
pub fn lenient_opt_f32<'de, D>(deserializer: D) -> Result<Option<f32>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Null => None,
        other => Some(coerce_f64(&other) as f32),
    })
}

/// `#[serde(deserialize_with = "coerce::lenient_opt_bool")]` — accept a bool,
/// the strings "true"/"false"/"1"/"0"/"yes"/"no"/"y"/"n"/"on"/"off"
/// (case-insensitive), or a number (non-zero → true). `null`/absent → `None`.
/// Anything else → `None`. Never errors.
pub fn lenient_opt_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde_json::Value;
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        Value::Bool(b) => Some(b),
        Value::String(s) => Some(matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "y" | "on"
        )),
        Value::Number(n) => Some(n.as_f64().is_some_and(|f| f != 0.0)),
        // `null`, absent, and any other shape → None.
        _ => None,
    })
}

/// `#[serde(deserialize_with = "coerce::lenient_opt_f32_map")]` — accept a JSON
/// object of `{ string: number }` and coerce each value forgivingly to `f32`
/// (numeric strings allowed). `null`/absent or a non-object → `None`. Never
/// errors, so a malformed `corpus_boost` can't sink the whole `survey` call.
#[allow(
    clippy::cast_possible_truncation,
    reason = "f64→f32 narrowing is acceptable for a boost multiplier"
)]
pub fn lenient_opt_f32_map<'de, D>(
    deserializer: D,
) -> Result<Option<HashMap<String, f32>>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde_json::Value;
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        Value::Object(map) => Some(
            map.into_iter()
                .map(|(k, v)| (k, coerce_f64(&v) as f32))
                .collect(),
        ),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Strs {
        #[serde(default, deserialize_with = "lenient_string")]
        s: String,
        #[serde(default, deserialize_with = "lenient_opt_string")]
        os: Option<String>,
        #[serde(default, deserialize_with = "string_or_seq")]
        seq: Vec<String>,
        #[serde(default, deserialize_with = "opt_string_or_seq")]
        oseq: Option<Vec<String>>,
    }

    fn strs(json: &str) -> Strs {
        serde_json::from_str(json).expect("forgiving args never reject")
    }

    #[test]
    fn lenient_string_handles_missing_null_and_scalars() {
        assert_eq!(strs(r"{}").s, "");
        assert_eq!(strs(r#"{"s":null}"#).s, "");
        assert_eq!(strs(r#"{"s":"hi"}"#).s, "hi");
        assert_eq!(strs(r#"{"s":42}"#).s, "42");
        assert_eq!(strs(r#"{"s":true}"#).s, "true");
    }

    #[test]
    fn lenient_opt_string_distinguishes_empty_and_absent() {
        assert_eq!(strs(r"{}").os, None);
        assert_eq!(strs(r#"{"os":null}"#).os, None);
        assert_eq!(strs(r#"{"os":""}"#).os, None);
        assert_eq!(strs(r#"{"os":"x"}"#).os, Some("x".to_string()));
    }

    #[test]
    fn string_or_seq_accepts_single_array_and_scalars() {
        assert_eq!(strs(r#"{"seq":"only"}"#).seq, vec!["only"]);
        assert_eq!(strs(r#"{"seq":["a","b"]}"#).seq, vec!["a", "b"]);
        assert!(strs(r"{}").seq.is_empty());
        assert!(strs(r#"{"seq":null}"#).seq.is_empty());
        assert_eq!(strs(r#"{"seq":[1,true]}"#).seq, vec!["1", "true"]);
    }

    #[test]
    fn opt_string_or_seq_keeps_null_as_none() {
        assert_eq!(strs(r"{}").oseq, None);
        assert_eq!(strs(r#"{"oseq":null}"#).oseq, None);
        assert_eq!(strs(r#"{"oseq":"x"}"#).oseq, Some(vec!["x".to_string()]));
        assert_eq!(strs(r#"{"oseq":[]}"#).oseq, Some(vec![]));
    }

    #[derive(Deserialize)]
    struct Nums {
        #[serde(default, deserialize_with = "lenient_opt_usize")]
        u: Option<usize>,
        #[serde(default, deserialize_with = "lenient_opt_u32")]
        u32v: Option<u32>,
        #[serde(default, deserialize_with = "lenient_opt_f32")]
        f: Option<f32>,
        #[serde(default, deserialize_with = "lenient_opt_bool")]
        b: Option<bool>,
        #[serde(default, deserialize_with = "lenient_opt_f32_map")]
        m: Option<HashMap<String, f32>>,
    }

    fn nums(json: &str) -> Nums {
        serde_json::from_str(json).expect("forgiving args never reject")
    }

    #[test]
    fn lenient_opt_numbers_accept_ints_strings_and_fall_back() {
        assert_eq!(nums(r"{}").u, None);
        assert_eq!(nums(r#"{"u":5}"#).u, Some(5));
        assert_eq!(nums(r#"{"u":"7"}"#).u, Some(7));
        assert_eq!(nums(r#"{"u":"nonsense"}"#).u, Some(0));
        assert_eq!(nums(r#"{"u32v":"3"}"#).u32v, Some(3));
        assert_eq!(nums(r#"{"u32v":true}"#).u32v, Some(0));
    }

    #[test]
    fn lenient_opt_f32_accepts_number_string_and_rejects_nonfinite() {
        assert_eq!(nums(r#"{"f":0.5}"#).f, Some(0.5));
        assert_eq!(nums(r#"{"f":"0.25"}"#).f, Some(0.25));
        assert_eq!(nums(r#"{"f":"x"}"#).f, Some(0.0));
        assert_eq!(nums(r"{}").f, None);
    }

    #[test]
    fn lenient_opt_bool_accepts_many_shapes() {
        assert_eq!(nums(r#"{"b":true}"#).b, Some(true));
        assert_eq!(nums(r#"{"b":"yes"}"#).b, Some(true));
        assert_eq!(nums(r#"{"b":"0"}"#).b, Some(false));
        assert_eq!(nums(r#"{"b":1}"#).b, Some(true));
        assert_eq!(nums(r"{}").b, None);
        assert_eq!(nums(r#"{"b":null}"#).b, None);
    }

    #[test]
    fn lenient_opt_f32_map_coerces_values_and_skips_non_objects() {
        let m = nums(r#"{"m":{"a":2.0,"b":"3"}}"#).m.unwrap();
        assert_eq!(m.get("a"), Some(&2.0));
        assert_eq!(m.get("b"), Some(&3.0));
        assert_eq!(nums(r#"{"m":"oops"}"#).m, None);
        assert_eq!(nums(r"{}").m, None);
    }
}

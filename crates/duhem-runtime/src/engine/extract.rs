//! Functional `outputs:` extraction (spec #273).
//!
//! `Step.outputs` maps a local alias to an extraction path into the
//! step's raw action result — `http_code: status` (rename) or
//! `project_id: body.data._id` (derived extraction). This module turns
//! that path plus the step's raw `BTreeMap<String, serde_json::Value>`
//! outputs into the extracted JSON value; the runner and setup walker
//! then record it under the alias so `$steps.<id>.outputs.<local>` (or
//! `$setup.<id>.outputs.<local>`) resolves.
//!
//! The grammar mirrors the evaluator's `navigate` (`eval.rs`): dotted
//! object keys and `[N]` array indices, with the first segment naming a
//! raw output field. Key-vs-index is disambiguated by the value's shape
//! (a numeric segment indexes an `Array` but is an ordinary key on an
//! `Object`). A miss — absent key, out-of-range index, or a descent
//! into a scalar — returns `None`, so the alias is simply not recorded
//! and a reference to it is `Inconclusive: MissingObservation`, the
//! same contract as any output an action did not produce.

use std::collections::BTreeMap;

use serde_json::Value as Json;

use crate::engine::context::json_to_value;
use crate::eval::Value;

/// Record a step's outputs into the check context via `record`: first
/// every raw action field under its native name, then each `outputs:`
/// alias (spec #273). Shared by the per-check (`runner.rs`) and setup
/// (`setup.rs`) paths so both bind identically. A raw field or an alias
/// whose value falls outside the `Value` model — or an alias whose
/// extraction misses — is skipped and simply not recorded.
pub(crate) fn record_step_outputs(
    outputs_map: &BTreeMap<String, String>,
    raw: &BTreeMap<String, Json>,
    mut record: impl FnMut(&str, Value),
) {
    for (name, value) in raw {
        if let Some(v) = json_to_value(value) {
            record(name, v);
        }
    }
    for (local, extraction) in outputs_map {
        if let Some(extracted) = resolve(raw, extraction)
            && let Some(v) = json_to_value(&extracted)
        {
            record(local, v);
        }
    }
}

/// Navigate `path` into a step's raw `outputs` and return the extracted
/// value. `None` when any segment misses. See the module docs for the
/// grammar.
pub(crate) fn resolve(outputs: &BTreeMap<String, Json>, path: &str) -> Option<Json> {
    let segments = lower(path);
    let (head, rest) = segments.split_first()?;
    let mut cur = outputs.get(head)?;
    for seg in rest {
        cur = match cur {
            Json::Object(map) => map.get(seg)?,
            Json::Array(items) => items.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur.clone())
}

/// Lower a dotted extraction path into navigation segments, peeling
/// `[N]` array indices into their own segments so they match the
/// evaluator's path lowering: `body.items[0].id` becomes
/// `["body", "items", "0", "id"]`.
fn lower(path: &str) -> Vec<String> {
    let mut segments = Vec::new();
    for dotted in path.split('.') {
        let key_end = dotted.find('[').unwrap_or(dotted.len());
        let key = &dotted[..key_end];
        if !key.is_empty() {
            segments.push(key.to_string());
        }
        // Trailing `[N][M]…` groups on this chunk each become a segment.
        let mut rest = &dotted[key_end..];
        while rest.starts_with('[') {
            match rest.find(']') {
                Some(close) => {
                    segments.push(rest[1..close].to_string());
                    rest = &rest[close + 1..];
                }
                None => break,
            }
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn outputs() -> BTreeMap<String, Json> {
        let mut m = BTreeMap::new();
        m.insert("status".to_string(), json!(200));
        m.insert(
            "body".to_string(),
            json!({ "data": { "_id": "abc123" }, "items": [{ "id": "a" }, { "id": "b" }] }),
        );
        m
    }

    #[test]
    fn rename_top_level_field() {
        // `http_code: status` — alias a raw field under a new name.
        assert_eq!(resolve(&outputs(), "status"), Some(json!(200)));
    }

    #[test]
    fn derived_extraction_deep_object() {
        // `project_id: body.data._id` — pluck a nested value.
        assert_eq!(resolve(&outputs(), "body.data._id"), Some(json!("abc123")));
    }

    #[test]
    fn array_index_then_key() {
        // `first_id: body.items[0].id` — index into an array, then a key.
        assert_eq!(resolve(&outputs(), "body.items[0].id"), Some(json!("a")));
        assert_eq!(resolve(&outputs(), "body.items[1].id"), Some(json!("b")));
    }

    #[test]
    fn missing_head_field_is_none() {
        assert_eq!(resolve(&outputs(), "nope"), None);
    }

    #[test]
    fn missing_nested_key_is_none() {
        assert_eq!(resolve(&outputs(), "body.data.absent"), None);
    }

    #[test]
    fn index_out_of_range_is_none() {
        assert_eq!(resolve(&outputs(), "body.items[9].id"), None);
    }

    #[test]
    fn descent_into_scalar_is_none() {
        // `status` is a scalar; navigating past it misses rather than
        // erroring, mirroring the evaluator's `NotNavigable` → no value.
        assert_eq!(resolve(&outputs(), "status.foo"), None);
    }

    #[test]
    fn empty_path_is_none() {
        assert_eq!(resolve(&outputs(), ""), None);
    }

    #[test]
    fn numeric_segment_on_object_is_a_key_not_an_index() {
        // Shape disambiguation: on an Object a digit segment is a key.
        let mut m = BTreeMap::new();
        m.insert("headers".to_string(), json!({ "0": "zero" }));
        assert_eq!(resolve(&m, "headers.0"), Some(json!("zero")));
    }
}

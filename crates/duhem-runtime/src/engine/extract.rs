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

/// The `capture/` output-name prefix is reserved for runner-emitted
/// evidence (spec §7.7 / #202). `duhem-schema::validate` rejects an
/// authored alias under it, but this crate never calls `validate` —
/// the CLI does, on whatever path constructed the definition, and a
/// programmatically-built definition can skip that call entirely. This
/// is the runtime backstop: independent of validation, an alias under
/// this prefix is never bound (#532).
const RESERVED_OUTPUT_PREFIX: &str = "capture/";

/// Record a step's outputs into the check context via `record`: first
/// every raw action field under its native name, then each `outputs:`
/// alias (spec #273). Shared by the per-check (`runner.rs`) and setup
/// (`setup.rs`) paths so both bind identically. A raw field or an alias
/// whose value falls outside the `Value` model — or an alias whose
/// extraction misses — is skipped and simply not recorded.
///
/// Raw fields are the runner's own vocabulary (no action produces a
/// `capture/*` field, so this loop never sees the reserved prefix in
/// practice) and are always bound. Authored `outputs:` aliases are the
/// surface #532 hardens: an alias under [`RESERVED_OUTPUT_PREFIX`] is
/// refused rather than bound, regardless of whether the definition was
/// validated. Returns the refused alias names so the caller can record
/// the refusal as evidence instead of dropping it silently.
pub(crate) fn record_step_outputs(
    outputs_map: &BTreeMap<String, String>,
    raw: &BTreeMap<String, Json>,
    mut record: impl FnMut(&str, Value),
) -> Vec<String> {
    for (name, value) in raw {
        if let Some(v) = json_to_value(value) {
            record(name, v);
        }
    }
    let mut refused = Vec::new();
    for (local, extraction) in outputs_map {
        if local.starts_with(RESERVED_OUTPUT_PREFIX) {
            refused.push(local.clone());
            continue;
        }
        if let Some(extracted) = resolve(raw, extraction)
            && let Some(v) = json_to_value(&extracted)
        {
            record(local, v);
        }
    }
    refused
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

    // #532: runtime backstop for the reserved `capture/` output-name
    // prefix. `record_step_outputs` never calls `duhem_schema::validate`
    // (the CLI does, on whatever path built the definition) so it must
    // refuse a reserved alias on its own, regardless of whether that
    // definition was ever validated.

    #[test]
    fn reserved_alias_is_refused_even_when_validation_was_bypassed() {
        // A step's `outputs:` built directly here, never run through
        // `duhem_schema::validate` — the scenario a programmatically
        // constructed definition can hit.
        let mut outputs_map = BTreeMap::new();
        outputs_map.insert("capture/evil".to_string(), "status".to_string());
        let mut recorded: Vec<(String, Value)> = Vec::new();
        let refused = record_step_outputs(&outputs_map, &outputs(), |name, v| {
            recorded.push((name.to_string(), v));
        });
        assert_eq!(
            refused,
            vec!["capture/evil".to_string()],
            "the reserved alias must be reported as refused"
        );
        assert!(
            !recorded.iter().any(|(name, _)| name == "capture/evil"),
            "the reserved alias must never be bound: {recorded:?}"
        );
    }

    #[test]
    fn legitimate_raw_output_under_capture_prefix_is_still_recorded() {
        // Negative control: the guard must discriminate an *authored*
        // alias from the runner's own raw-field vocabulary, not ban the
        // `capture/` namespace outright. No action actually emits a raw
        // field under this prefix (real captures ride a separate
        // evidence channel — see `engine::capture`), but this proves
        // the discrimination is structural (which loop the name comes
        // through), not a blanket string-prefix ban that would also
        // reject a legitimate runner-emitted observation.
        let mut raw = outputs();
        raw.insert("capture/screenshot".to_string(), json!("ok"));
        let mut recorded: Vec<(String, Value)> = Vec::new();
        let refused = record_step_outputs(&BTreeMap::new(), &raw, |name, v| {
            recorded.push((name.to_string(), v))
        });
        assert!(refused.is_empty(), "no alias was authored: {refused:?}");
        assert!(
            recorded
                .iter()
                .any(|(name, _)| name == "capture/screenshot"),
            "a legitimate raw `capture/*` field must still be recorded: {recorded:?}"
        );
    }

    #[test]
    fn ordinary_alias_is_unaffected() {
        let mut outputs_map = BTreeMap::new();
        outputs_map.insert("http_code".to_string(), "status".to_string());
        let mut recorded: Vec<(String, Value)> = Vec::new();
        let refused = record_step_outputs(&outputs_map, &outputs(), |name, v| {
            recorded.push((name.to_string(), v));
        });
        assert!(refused.is_empty());
        assert!(recorded.iter().any(|(name, _)| name == "http_code"));
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

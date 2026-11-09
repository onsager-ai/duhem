//! Static checks for the `api/observe` `window:` field (#603 P3).
//!
//! The action deserializes the same shape at run time; checking it here
//! turns a malformed window into an authoring-time error instead of a
//! step failure mid-run. `edge` and `duration` must be literals.

use serde_yml::Value;

/// `None` when the window is well formed. `is_earlier` answers whether a
/// step id names an earlier step of the same check.
pub(crate) fn problem(window: &Value, is_earlier: impl Fn(&str) -> bool) -> Option<String> {
    let Some(map) = window.as_mapping() else {
        return Some("window must be a mapping of from_step, edge and duration".into());
    };
    match map.get("from_step").and_then(Value::as_str) {
        None => return Some("window.from_step is required".into()),
        Some(id) if !is_earlier(id) => {
            return Some("window.from_step must name an earlier step in the same check".into());
        }
        Some(_) => {}
    }
    if let Some(edge) = map.get("edge")
        && !matches!(edge.as_str(), Some("start" | "finish"))
    {
        return Some("window.edge must be `start` or `finish`".into());
    }
    match map.get("duration") {
        None => Some("window.duration is required".into()),
        Some(duration) if !positive_duration(duration) => {
            Some("window.duration must be a positive duration like `500ms`, `2s` or `1m`".into())
        }
        Some(_) => None,
    }
}

fn positive_duration(value: &Value) -> bool {
    match value {
        Value::Number(n) => n.as_u64().is_some_and(|ms| ms > 0),
        Value::String(raw) => {
            let raw = raw.trim();
            let digits = ["ms", "s", "m"]
                .iter()
                .find_map(|suffix| raw.strip_suffix(suffix))
                .map(str::trim);
            digits
                .and_then(|n| n.parse::<u64>().ok())
                .is_some_and(|n| n > 0)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::validate::{ValidationError, validate};
    use crate::verification::VerificationDefinition;

    fn errors(window: &str, body: &str) -> Vec<String> {
        let yaml = format!(
            r#"
verification: x
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps:
          - id: load
            uses: ui/navigate
            with: {{ url: "http://x/" }}
{body}
          - id: reads
            uses: api/observe
            with:
              url_pattern: "re:.*"
              window: {window}
        assertions: ["true"]
"#
        );
        let v = VerificationDefinition::from_yaml_str(&yaml).expect("parse");
        match validate(&v) {
            Ok(()) => vec![],
            Err(errs) => errs
                .into_iter()
                .filter_map(|e| match e {
                    ValidationError::InvalidFlow { message } => Some(message),
                    _ => None,
                })
                .collect(),
        }
    }

    fn rejected(window: &str, needle: &str) {
        let errs = errors(window, "");
        assert!(
            errs.iter().any(|m| m.contains(needle)),
            "{window}: {errs:?}"
        );
    }

    #[test]
    fn well_formed_windows_pass() {
        for window in [
            "{ from_step: load, duration: 1s }",
            "{ from_step: load, edge: finish, duration: 250 }",
            "{ from_step: load, edge: start, duration: 2m }",
        ] {
            assert_eq!(errors(window, ""), Vec::<String>::new(), "{window}");
        }
    }

    #[test]
    fn malformed_windows_are_rejected() {
        rejected("{ duration: 1s }", "from_step is required");
        rejected("{ from_step: nope, duration: 1s }", "earlier step");
        rejected("{ from_step: reads, duration: 1s }", "earlier step");
        rejected("{ from_step: load, edge: middle, duration: 1s }", "edge");
        rejected("{ from_step: load }", "duration is required");
        rejected("{ from_step: load, duration: 0ms }", "positive");
        rejected("{ from_step: load, duration: 0 }", "positive");
        rejected("{ from_step: load, duration: -5 }", "positive");
        rejected("{ from_step: load, duration: -1s }", "positive");
        rejected("{ from_step: load, duration: soon }", "positive");
        rejected("not-a-mapping", "mapping");
    }

    #[test]
    fn anchor_must_precede_inside_for_each_bodies() {
        let yaml = |anchor: &str| {
            format!(
                r#"
verification: x
inputs: {{ rows: {{ type: array }} }}
criteria:
  - id: AC-1
    description: a
    checks:
      - id: AC-1.1
        steps:
          - id: load
            uses: ui/navigate
            with: {{ url: "http://x/" }}
          - for_each: $inputs.rows
            max: 2
            as: row
            steps:
              - id: reads
                uses: api/observe
                with:
                  url_pattern: "re:.*"
                  window: {{ from_step: {anchor}, duration: 1s }}
        assertions: ["true"]
"#
            )
        };
        let check = |anchor: &str| {
            validate(&VerificationDefinition::from_yaml_str(&yaml(anchor)).expect("parse"))
        };
        assert!(check("load").is_ok());
        let errs = check("later").unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ValidationError::InvalidFlow { message } if message.contains("earlier step")
        )));
    }
}

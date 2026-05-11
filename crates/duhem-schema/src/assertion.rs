//! `Assertion` — the closed set of mechanically-judgable claims a check
//! makes. The closed-enum shape is the structural enforcement of the
//! "mechanical judgment, not LLM judgment" identity commitment
//! (`CLAUDE.md`): there is no free-text "let-the-LLM-decide" variant,
//! and a judge implementing this enum cannot accidentally call out to
//! a model.
//!
//! YAML shape, per `docs/duhem-spec.md` §10.6:
//!
//! ```yaml
//! - $steps.api.outputs.status == 200            # bare expression
//! - type_check: { value: $..., is: uuid }
//! - matches:    { value: $..., pattern: "^.*$" }
//! - in:         { value: $..., set: [1, 2, 3] }
//! - exists: $steps.x.outputs.y
//! - equal:  [$a, $b]
//! ```

use serde::de::{Deserializer, Error as DeError};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};

use crate::expr::{self, ExprStr};

#[derive(Debug, Clone, PartialEq)]
pub enum Assertion {
    /// Bare boolean expression. Round-tripped via `ExprStr.raw`.
    Expr(ExprStr),
    TypeCheck {
        value: ExprStr,
        is: TypeCheckKind,
    },
    Matches {
        value: ExprStr,
        pattern: String,
    },
    In {
        value: ExprStr,
        set: Vec<serde_yml::Value>,
    },
    Exists {
        value: ExprStr,
    },
    Equal {
        values: Vec<ExprStr>,
    },
}

/// The closed set of structural type names recognized by `type_check`.
/// Extending this is a v0.x breaking change and goes through the
/// schema-impact gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeCheckKind {
    Uuid,
    String,
    Integer,
    Float,
    Boolean,
    Object,
    Array,
    Null,
}

impl Assertion {
    /// Walk every `ExprStr` this assertion holds. Used by the validator
    /// to resolve all `$steps.*` and `$inputs.*` references.
    pub fn walk_exprs<F: FnMut(&ExprStr)>(&self, mut visit: F) {
        match self {
            Assertion::Expr(e)
            | Assertion::Exists { value: e }
            | Assertion::TypeCheck { value: e, .. }
            | Assertion::Matches { value: e, .. }
            | Assertion::In { value: e, .. } => visit(e),
            Assertion::Equal { values } => {
                for v in values {
                    visit(v);
                }
            }
        }
    }
}

impl Serialize for Assertion {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        match self {
            Assertion::Expr(e) => ser.serialize_str(&e.raw),
            Assertion::TypeCheck { value, is } => {
                #[derive(Serialize)]
                struct Body<'a> {
                    value: &'a ExprStr,
                    is: &'a TypeCheckKind,
                }
                ser_single(ser, "type_check", &Body { value, is })
            }
            Assertion::Matches { value, pattern } => {
                #[derive(Serialize)]
                struct Body<'a> {
                    value: &'a ExprStr,
                    pattern: &'a str,
                }
                ser_single(ser, "matches", &Body { value, pattern })
            }
            Assertion::In { value, set } => {
                #[derive(Serialize)]
                struct Body<'a> {
                    value: &'a ExprStr,
                    set: &'a Vec<serde_yml::Value>,
                }
                ser_single(ser, "in", &Body { value, set })
            }
            Assertion::Exists { value } => ser_single(ser, "exists", &value.raw),
            Assertion::Equal { values } => ser_single(ser, "equal", values),
        }
    }
}

fn ser_single<S, V>(ser: S, key: &str, value: V) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    V: Serialize,
{
    let mut m = ser.serialize_map(Some(1))?;
    m.serialize_entry(key, &value)?;
    m.end()
}

impl<'de> Deserialize<'de> for Assertion {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let v = serde_yml::Value::deserialize(de)?;
        match v {
            serde_yml::Value::String(s) => {
                let parsed = expr::parse(&s).map_err(D::Error::custom)?;
                Ok(Assertion::Expr(ExprStr { raw: s, parsed }))
            }
            serde_yml::Value::Mapping(m) => {
                if m.len() != 1 {
                    return Err(D::Error::custom(
                        "assertion mapping must have exactly one key (type_check, matches, in, exists, or equal)",
                    ));
                }
                let (k, body) = m.into_iter().next().unwrap();
                let key = k
                    .as_str()
                    .ok_or_else(|| D::Error::custom("assertion key must be a string"))?;
                deserialize_keyed(key, body).map_err(D::Error::custom)
            }
            other => Err(D::Error::custom(format!(
                "assertion must be a string or single-key mapping, got {other:?}"
            ))),
        }
    }
}

fn deserialize_keyed(key: &str, body: serde_yml::Value) -> Result<Assertion, String> {
    match key {
        "type_check" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Body {
                value: ExprStr,
                is: TypeCheckKind,
            }
            let b: Body = serde_yml::from_value(body).map_err(|e| e.to_string())?;
            Ok(Assertion::TypeCheck {
                value: b.value,
                is: b.is,
            })
        }
        "matches" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Body {
                value: ExprStr,
                pattern: String,
            }
            let b: Body = serde_yml::from_value(body).map_err(|e| e.to_string())?;
            Ok(Assertion::Matches {
                value: b.value,
                pattern: b.pattern,
            })
        }
        "in" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Body {
                value: ExprStr,
                set: Vec<serde_yml::Value>,
            }
            let b: Body = serde_yml::from_value(body).map_err(|e| e.to_string())?;
            Ok(Assertion::In {
                value: b.value,
                set: b.set,
            })
        }
        "exists" => {
            let s: String = serde_yml::from_value(body).map_err(|e| e.to_string())?;
            let parsed = expr::parse(&s).map_err(|e| e.to_string())?;
            Ok(Assertion::Exists {
                value: ExprStr { raw: s, parsed },
            })
        }
        "equal" => {
            let vs: Vec<ExprStr> = serde_yml::from_value(body).map_err(|e| e.to_string())?;
            Ok(Assertion::Equal { values: vs })
        }
        other => Err(format!(
            "unknown assertion form `{other}` (expected: type_check, matches, in, exists, equal, or bare expression)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_expression() {
        let y = "$steps.api.outputs.status == 200\n";
        let a: Assertion = serde_yml::from_str(y).expect("parse");
        match a {
            Assertion::Expr(e) => assert_eq!(e.raw, "$steps.api.outputs.status == 200"),
            other => panic!("expected Expr, got {other:?}"),
        }
    }

    #[test]
    fn parses_type_check() {
        let y = "type_check: { value: $steps.x.outputs.y, is: uuid }\n";
        let a: Assertion = serde_yml::from_str(y).expect("parse");
        match a {
            Assertion::TypeCheck {
                is: TypeCheckKind::Uuid,
                ..
            } => {}
            other => panic!("expected TypeCheck, got {other:?}"),
        }
    }

    #[test]
    fn parses_matches() {
        let y = "matches: { value: $inputs.x, pattern: \".*\" }\n";
        let a: Assertion = serde_yml::from_str(y).expect("parse");
        assert!(matches!(a, Assertion::Matches { .. }));
    }

    #[test]
    fn parses_in() {
        let y = "in: { value: $inputs.x, set: [1, 2, 3] }\n";
        let a: Assertion = serde_yml::from_str(y).expect("parse");
        match a {
            Assertion::In { set, .. } => assert_eq!(set.len(), 3),
            other => panic!("expected In, got {other:?}"),
        }
    }

    #[test]
    fn parses_exists() {
        let y = "exists: $steps.api.outputs.id\n";
        let a: Assertion = serde_yml::from_str(y).expect("parse");
        assert!(matches!(a, Assertion::Exists { .. }));
    }

    #[test]
    fn parses_equal() {
        let y = "equal: [$inputs.a, $inputs.b]\n";
        let a: Assertion = serde_yml::from_str(y).expect("parse");
        match a {
            Assertion::Equal { values } => assert_eq!(values.len(), 2),
            other => panic!("expected Equal, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_form() {
        let y = "wat: { value: $inputs.x }\n";
        assert!(serde_yml::from_str::<Assertion>(y).is_err());
    }

    #[test]
    fn rejects_multi_key_mapping() {
        let y = "type_check: { value: $inputs.x, is: uuid }\nexists: $inputs.x\n";
        assert!(serde_yml::from_str::<Assertion>(y).is_err());
    }

    #[test]
    fn rejects_bad_expression_in_value() {
        let y = "type_check: { value: $nope.x, is: uuid }\n";
        let err = serde_yml::from_str::<Assertion>(y).unwrap_err();
        assert!(format!("{err}").contains("unknown scope"), "got: {err}");
    }

    #[test]
    fn round_trip_bare_expression() {
        let a = Assertion::Expr(ExprStr::from_source("$inputs.x == 1").unwrap());
        let y = serde_yml::to_string(&a).unwrap();
        assert!(y.contains("$inputs.x == 1"), "got: {y}");
        let back: Assertion = serde_yml::from_str(&y).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn round_trip_type_check() {
        let a = Assertion::TypeCheck {
            value: ExprStr::from_source("$steps.x.outputs.id").unwrap(),
            is: TypeCheckKind::Uuid,
        };
        let y = serde_yml::to_string(&a).unwrap();
        let back: Assertion = serde_yml::from_str(&y).unwrap();
        assert_eq!(a, back);
    }
}

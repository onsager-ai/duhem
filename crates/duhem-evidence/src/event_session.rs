//! Lifecycle seed metadata is carried by the envelope so payload constructors
//! and existing lifecycle readers remain source-compatible.
use crate::{Event, EventPayload};
use serde::{Deserialize, Deserializer};

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Preserve derive's duplicate-field rejection before using a JSON map
        // to separate lifecycle metadata from the existing flattened payload.
        struct Fields;
        impl<'de> serde::de::Visitor<'de> for Fields {
            type Value = serde_json::Value;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an event object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                let mut fields = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, serde_json::Value>()? {
                    if fields.contains_key(&key) {
                        return Err(serde::de::Error::custom(format!("duplicate field `{key}`")));
                    }
                    fields.insert(key, value);
                }
                Ok(fields.into())
            }
        }
        let mut value = deserializer.deserialize_map(Fields)?;
        let (session_source, session_digest) = if value["kind"] == "setup_started" {
            let object = value.as_object_mut().expect("kind belongs to an object");
            (
                object.remove("session_source"),
                object.remove("session_digest"),
            )
        } else {
            (None, None)
        };
        #[derive(Deserialize)]
        struct Core {
            #[serde(default)]
            session: Option<String>,
            seq: u64,
            #[serde(with = "crate::event::ts_ms")]
            ts: chrono::DateTime<chrono::Utc>,
            #[serde(flatten)]
            payload: EventPayload,
        }
        let core: Core = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            session_source,
            session_digest,
            session: core.session,
            seq: core.seq,
            ts: core.ts,
            payload: core.payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::Event;

    #[test]
    fn lifecycle_seed_fields_round_trip_without_changing_payload_fields() {
        for sources in [
            serde_json::json!(null),
            serde_json::json!("$inputs.state"),
            serde_json::json!({"actor": "$inputs.state", "visitor": null}),
        ] {
            let wire = serde_json::json!({"seq": 1, "ts": "2026-09-23T00:00:00.000Z", "kind": "setup_started", "step_count": 1, "session_source": sources, "session_digest": null});
            let event: Event = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(event.session_source, Some(sources));
            assert_eq!(serde_json::to_value(event).unwrap(), wire);
        }
    }

    #[test]
    fn duplicate_event_fields_remain_invalid() {
        let wire = r#"{"seq":1,"seq":2,"ts":"2026-09-23T00:00:00.000Z","kind":"run_heartbeat"}"#;
        assert!(
            serde_json::from_str::<Event>(wire)
                .unwrap_err()
                .to_string()
                .contains("duplicate field")
        );
    }
}

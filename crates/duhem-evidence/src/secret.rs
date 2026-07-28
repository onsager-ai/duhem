//! Secret-value registration and evidence-boundary masking (spec #346).
//!
//! Actions never decide what is sensitive. Input resolution registers
//! secret values here, then the evidence writer applies one exact,
//! case-sensitive Aho-Corasick pass at the sink. That placement is the
//! inheritance guarantee: a future action family reaches the same event
//! and blob writer without learning that masking exists.
//!
//! The registry also includes the three transformations most often made
//! in transit: standard base64, percent encoding, and JSON string
//! escaping. More distant transformations (hashing, chunking, or
//! application-specific rewriting) deliberately remain outside the
//! substring guarantee.

use std::collections::BTreeMap;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use base64::Engine as _;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

/// Runtime warning threshold from spec #346. It is intentionally a
/// visible default rather than a validation floor: short fixtures remain
/// legal, but authors are warned before over-masking durable evidence.
pub const SHORT_SECRET_THRESHOLD: usize = 8;

/// Small, explicit collision-prone fixture vocabulary. This is a
/// tunable default, not an attempt at a language dictionary.
pub const COMMON_SECRET_WORDS: &[&str] =
    &["admin", "changeme", "password", "secret", "test", "token"];

/// Masked text plus the non-zero occurrence counts produced in the same
/// matcher pass. Counts are keyed by declared input name so evidence can
/// explain why an artifact looks sparse without revealing its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskedText {
    pub text: String,
    pub counts: BTreeMap<String, u64>,
}

/// A registry of sensitive value spellings and their owning input names.
///
/// Duplicate spellings are deterministic: the lexicographically first
/// input name owns the replacement token. That preserves the required
/// value → input-name mapping even when two declarations resolve to the
/// same credential.
#[derive(Clone, Default)]
pub struct SecretRegistry {
    patterns: BTreeMap<String, String>,
    matcher: Option<AhoCorasick>,
    matcher_names: Vec<String>,
    risky: BTreeMap<String, Vec<&'static str>>,
}

impl SecretRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Register one resolved secret value and its common encoded forms.
    ///
    /// Empty values still participate in risk warnings, but have no
    /// substring occurrences to replace. Registering an empty pattern
    /// would manufacture matches between every pair of characters,
    /// which is not evidence of the value appearing.
    pub fn register(&mut self, input_name: impl Into<String>, value: &str) {
        let input_name = input_name.into();
        let mut reasons = Vec::new();
        if value.chars().count() < SHORT_SECRET_THRESHOLD {
            reasons.push("shorter than 8 characters");
        }
        if COMMON_SECRET_WORDS
            .iter()
            .any(|word| value.eq_ignore_ascii_case(word))
        {
            reasons.push("a common fixture word");
        }
        if !reasons.is_empty() {
            self.risky.insert(input_name.clone(), reasons);
        }

        if value.is_empty() {
            return;
        }

        let json = serde_json::to_string(value).expect("string serializes");
        let json_escaped = &json[1..json.len() - 1];
        let forms = [
            value.to_string(),
            base64::engine::general_purpose::STANDARD.encode(value),
            utf8_percent_encode(value, NON_ALPHANUMERIC).to_string(),
            json_escaped.to_string(),
        ];
        for form in forms {
            if !form.is_empty() {
                self.patterns
                    .entry(form)
                    .and_modify(|owner| {
                        if input_name < *owner {
                            *owner = input_name.clone();
                        }
                    })
                    .or_insert_with(|| input_name.clone());
            }
        }
        self.rebuild();
    }

    /// Register the stable textual form of a resolved JSON input.
    pub fn register_json(&mut self, input_name: impl Into<String>, value: &serde_json::Value) {
        let text = match value {
            serde_json::Value::String(s) => s.clone(),
            other => serde_json::to_string(other).expect("JSON value serializes"),
        };
        self.register(input_name, &text);
    }

    /// Human-readable warnings for collision-prone declarations. Values
    /// are never included in the warning.
    pub fn warnings(&self) -> Vec<String> {
        self.risky
            .iter()
            .map(|(name, reasons)| {
                format!(
                    "secret input `{name}` is {}; exact-substring masking may over-mask evidence",
                    reasons.join(" and ")
                )
            })
            .collect()
    }

    /// Mask a text value in one Aho-Corasick pass.
    pub fn mask(&self, text: &str) -> MaskedText {
        let Some(matcher) = &self.matcher else {
            return MaskedText {
                text: text.to_string(),
                counts: BTreeMap::new(),
            };
        };
        let mut masked = String::with_capacity(text.len());
        let mut counts = BTreeMap::new();
        let mut end = 0;
        for found in matcher.find_iter(text) {
            masked.push_str(&text[end..found.start()]);
            let name = &self.matcher_names[found.pattern().as_usize()];
            masked.push_str("[redacted:");
            masked.push_str(name);
            masked.push(']');
            *counts.entry(name.clone()).or_insert(0) += 1;
            end = found.end();
        }
        masked.push_str(&text[end..]);
        MaskedText {
            text: masked,
            counts,
        }
    }

    /// Recursively mask textual leaves of a JSON value. This keeps
    /// structural event fields typed while covering every action-owned
    /// `with:` / output / response string that enters the generic event
    /// payload.
    pub fn mask_json(&self, value: &mut serde_json::Value) -> BTreeMap<String, u64> {
        let mut counts = BTreeMap::new();
        self.mask_json_into(value, &mut counts);
        counts
    }

    /// Mask a blob only when it is recorded text. Binary evidence is
    /// returned byte-for-byte; this is the explicit pixels/video limit.
    pub fn mask_bytes(&self, bytes: &[u8]) -> (Vec<u8>, BTreeMap<String, u64>) {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return (bytes.to_vec(), BTreeMap::new());
        };
        let masked = self.mask(text);
        (masked.text.into_bytes(), masked.counts)
    }

    fn mask_json_into(&self, value: &mut serde_json::Value, counts: &mut BTreeMap<String, u64>) {
        match value {
            serde_json::Value::String(text) => {
                let masked = self.mask(text);
                *text = masked.text;
                merge_counts(counts, masked.counts);
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    self.mask_json_into(value, counts);
                }
            }
            serde_json::Value::Object(map) => {
                for value in map.values_mut() {
                    self.mask_json_into(value, counts);
                }
            }
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            }
        }
    }

    fn rebuild(&mut self) {
        if self.patterns.is_empty() {
            self.matcher = None;
            self.matcher_names.clear();
            return;
        }
        let pattern_text: Vec<&str> = self.patterns.keys().map(String::as_str).collect();
        self.matcher_names = self.patterns.values().cloned().collect();
        self.matcher = Some(
            AhoCorasickBuilder::new()
                .match_kind(MatchKind::LeftmostLongest)
                .ascii_case_insensitive(false)
                .build(pattern_text)
                .expect("non-empty secret patterns compile"),
        );
    }
}

pub(crate) fn merge_counts(into: &mut BTreeMap<String, u64>, from: BTreeMap<String, u64>) {
    for (name, count) in from {
        *into.entry(name).or_insert(0) += count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_raw_and_common_encoded_forms_in_one_registry() {
        let mut registry = SecretRegistry::new();
        registry.register("db_dsn", "a/b:c");
        let base64 = base64::engine::general_purpose::STANDARD.encode("a/b:c");
        let text = format!("raw=a/b:c percent=a%2Fb%3Ac b64={base64}");
        let masked = registry.mask(&text);
        assert_eq!(
            masked.text,
            "raw=[redacted:db_dsn] percent=[redacted:db_dsn] b64=[redacted:db_dsn]"
        );
        assert_eq!(masked.counts.get("db_dsn"), Some(&3));
    }

    #[test]
    fn json_escaped_form_is_masked_without_breaking_json() {
        let mut registry = SecretRegistry::new();
        registry.register("api_key", "line\n\"quoted\"");
        let encoded = serde_json::to_string("line\n\"quoted\"").unwrap();
        let masked = registry.mask(&encoded);
        assert_eq!(masked.text, "\"[redacted:api_key]\"");
        assert_eq!(masked.counts.get("api_key"), Some(&1));
    }

    #[test]
    fn binary_blobs_are_not_treated_as_recorded_text() {
        let mut registry = SecretRegistry::new();
        registry.register("token", "secret");
        let bytes = [0xff, b's', b'e', b'c', b'r', b'e', b't'];
        let (masked, counts) = registry.mask_bytes(&bytes);
        assert_eq!(masked, bytes);
        assert!(counts.is_empty());
    }

    #[test]
    fn warnings_are_visible_but_do_not_block_registration() {
        let mut registry = SecretRegistry::new();
        registry.register("short", "abcd");
        registry.register("word", "TEST");
        registry.register("strong", "0123456789abcdef0123456789abcdef");
        assert_eq!(registry.warnings().len(), 2);
        assert!(registry.mask("abcd TEST").text.contains("[redacted:short]"));
    }
}

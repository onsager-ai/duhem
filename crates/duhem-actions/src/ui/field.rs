use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldSource {
    Attribute(String),
    Property(String),
    Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldSourceWire {
    #[serde(default)]
    field: Option<String>,
    #[serde(default)]
    attribute: Option<String>,
    #[serde(default)]
    property: Option<String>,
    #[serde(default)]
    text: Option<bool>,
}

impl<'de> Deserialize<'de> for FieldSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = FieldSourceWire::deserialize(deserializer)?;
        let count = usize::from(wire.field.is_some())
            + usize::from(wire.attribute.is_some())
            + usize::from(wire.property.is_some())
            + usize::from(wire.text.is_some());
        if count != 1 {
            return Err(serde::de::Error::custom(
                "field source requires exactly one of `field`, `attribute`, `property`, or `text`",
            ));
        }
        if let Some(name) = wire.field {
            return resolve_field(&name).map_err(serde::de::Error::custom);
        }
        if let Some(name) = wire.attribute {
            return Ok(Self::Attribute(name));
        }
        if let Some(name) = wire.property {
            return Ok(Self::Property(name));
        }
        match wire.text {
            Some(true) => Ok(Self::Text),
            Some(false) => Err(serde::de::Error::custom(
                "`text` must be true when selected",
            )),
            None => unreachable!(),
        }
    }
}

pub fn resolve_field(name: &str) -> Result<FieldSource, String> {
    match name {
        "checked" | "value" | "disabled" => Ok(FieldSource::Property(name.to_owned())),
        "text" => Ok(FieldSource::Text),
        "href" | "class" | "id" | "title" | "alt" => Ok(FieldSource::Attribute(name.to_owned())),
        name if name.starts_with("aria-") || name.starts_with("data-") => {
            Ok(FieldSource::Attribute(name.to_owned()))
        }
        _ => Err(format!(
            "unknown field `{name}`; use `attribute:` or `property:` to select an explicit source"
        )),
    }
}

impl FieldSource {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Attribute(_) => "attribute",
            Self::Property(_) => "property",
            Self::Text => "text",
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Attribute(name) | Self::Property(name) => Some(name),
            Self::Text => None,
        }
    }
}

/// Catalog-validation hook for the two actions sharing this vocabulary.
pub fn validate_field_source(uses: &str, with: &serde_yml::Value) -> Result<(), String> {
    let mapping = with
        .as_mapping()
        .ok_or_else(|| "`with:` must be a mapping".to_string())?;
    let source_value = if uses == "ui/assert-element" {
        let expected = mapping.contains_key(serde_yml::Value::String("expected".into()));
        let expect = mapping.get(serde_yml::Value::String("expect".into()));
        if expected == expect.is_some() {
            return Err("requires exactly one of `expected` or `expect`".into());
        }
        let Some(expect) = expect else { return Ok(()) };
        expect.clone()
    } else {
        with.clone()
    };
    let source_mapping = source_value
        .as_mapping()
        .ok_or_else(|| "field source must be a mapping".to_string())?;
    let mut only_source = serde_yml::Mapping::new();
    for name in ["field", "attribute", "property", "text"] {
        let key = serde_yml::Value::String(name.into());
        if let Some(value) = source_mapping.get(&key) {
            only_source.insert(key, value.clone());
        }
    }
    serde_yml::from_value::<FieldSource>(serde_yml::Value::Mapping(only_source))
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_field_table_and_explicit_sources() {
        assert_eq!(
            resolve_field("checked").unwrap(),
            FieldSource::Property("checked".into())
        );
        assert_eq!(
            resolve_field("aria-checked").unwrap(),
            FieldSource::Attribute("aria-checked".into())
        );
        assert!(resolve_field("mystery").unwrap_err().contains("attribute:"));
    }

    #[test]
    fn source_is_exactly_one_and_text_must_be_true() {
        assert!(serde_yml::from_str::<FieldSource>("{}").is_err());
        assert!(serde_yml::from_str::<FieldSource>("field: text\ntext: true").is_err());
        assert!(serde_yml::from_str::<FieldSource>("text: false").is_err());
    }
}

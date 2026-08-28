//! Browser viewport declaration shared by manifests and the CLI.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// CSS-pixel dimensions used by browser-backed checks in headless runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Viewport {
    #[serde(deserialize_with = "positive_dimension")]
    #[schemars(range(min = 1))]
    pub width: u32,
    #[serde(deserialize_with = "positive_dimension")]
    #[schemars(range(min = 1))]
    pub height: u32,
}

fn positive_dimension<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let value = u32::deserialize(d)?;
    if value == 0 {
        Err(serde::de::Error::custom(
            "viewport dimensions must be greater than zero",
        ))
    } else {
        Ok(value)
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
        }
    }
}

impl std::str::FromStr for Viewport {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (width, height) = value
            .split_once(['x', 'X'])
            .ok_or_else(|| "expected WIDTHxHEIGHT (for example 1440x900)".to_string())?;
        let width = width
            .parse::<u32>()
            .map_err(|_| "viewport width must be a positive integer".to_string())?;
        let height = height
            .parse::<u32>()
            .map_err(|_| "viewport height must be a positive integer".to_string())?;
        if width == 0 || height == 0 {
            return Err("viewport dimensions must be greater than zero".to_string());
        }
        Ok(Self { width, height })
    }
}

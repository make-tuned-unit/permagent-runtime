use std::{fmt, str::FromStr};

use anyhow::bail;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Harvest,
    Watch,
    Correlate,
    Draft,
    Critique,
}

impl Stage {
    pub const ALL: [Self; 5] = [
        Self::Harvest,
        Self::Watch,
        Self::Correlate,
        Self::Draft,
        Self::Critique,
    ];
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Harvest => "harvest",
            Self::Watch => "watch",
            Self::Correlate => "correlate",
            Self::Draft => "draft",
            Self::Critique => "critique",
        })
    }
}

impl FromStr for Stage {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "harvest" => Ok(Self::Harvest),
            "watch" => Ok(Self::Watch),
            "correlate" => Ok(Self::Correlate),
            "draft" => Ok(Self::Draft),
            "critique" => Ok(Self::Critique),
            _ => bail!(
                "unknown stage '{value}'; expected harvest, watch, correlate, draft, or critique"
            ),
        }
    }
}

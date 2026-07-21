use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    Portrait,
    Landscape,

    /// Forward-compatible fallback for unexpected strings.
    #[serde(other)]
    Unknown,
}

impl Default for Orientation {
    fn default() -> Self {
        Orientation::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SystemSettings {
    pub bluetooth_enabled: bool,
    pub location_enabled: bool,
    pub wifi_enabled: bool,
    pub orientation: Orientation,
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PadVelocityCurve {
    Soft3,
    Soft2,
    Soft1,
    Linear,
    Hard1,
    Hard2,
    Hard3,
}

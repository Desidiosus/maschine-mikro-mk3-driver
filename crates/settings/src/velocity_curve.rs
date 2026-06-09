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

impl std::fmt::Display for PadVelocityCurve {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PadVelocityCurve::Soft3 => "soft3",
            PadVelocityCurve::Soft2 => "soft2",
            PadVelocityCurve::Soft1 => "soft1",
            PadVelocityCurve::Linear => "linear",
            PadVelocityCurve::Hard1 => "hard1",
            PadVelocityCurve::Hard2 => "hard2",
            PadVelocityCurve::Hard3 => "hard3",
        };
        f.write_str(s)
    }
}

impl PadVelocityCurve {
    /// All variants, in display order (soft → hard).
    pub const ALL: [PadVelocityCurve; 7] = [
        PadVelocityCurve::Soft3,
        PadVelocityCurve::Soft2,
        PadVelocityCurve::Soft1,
        PadVelocityCurve::Linear,
        PadVelocityCurve::Hard1,
        PadVelocityCurve::Hard2,
        PadVelocityCurve::Hard3,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_matches_serde_token_for_all_variants() {
        for v in PadVelocityCurve::ALL {
            let token = serde_json::to_string(&v).unwrap();
            assert_eq!(v.to_string(), token.trim_matches('"'), "{v:?}");
        }
    }
}

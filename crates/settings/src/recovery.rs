//! `[recovery]`: USB auto-recovery switch and timing.

use serde::{Deserialize, Serialize};

macro_rules! recovery_settings {
    ($($field:ident: $ty:ty = $default:expr,)+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub struct RecoverySettings {
            $(pub $field: $ty,)+
        }

        impl Default for RecoverySettings {
            fn default() -> Self {
                Self { $($field: $default,)+ }
            }
        }

        #[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct PartialRecoverySettings {
            $(pub $field: Option<$ty>,)+
        }

        impl RecoverySettings {
            pub fn apply_overrides(&mut self, partial: PartialRecoverySettings) {
                $(if let Some(v) = partial.$field {
                    self.$field = v;
                })+
            }

            /// `None` when `self` matches `base`.
            pub fn diff_from(&self, base: &Self) -> Option<PartialRecoverySettings> {
                let partial = PartialRecoverySettings {
                    $($field: (self.$field != base.$field).then_some(self.$field),)+
                };
                (partial != PartialRecoverySettings::default()).then_some(partial)
            }
        }
    };
}

recovery_settings! {
    enabled: bool = true,
    quick_retries: u32 = 3,
    quick_retry_delay_ms: u64 = 5,
    reset_attempts: u32 = 2,
    reset_settle_ms: u64 = 500,
    backoff_initial_ms: u64 = 100,
    backoff_cap_ms: u64 = 2000,
    max_reopen_attempts: u32 = 6,
    max_consecutive_recoveries: u32 = 4,
    write_stall_timeout_ms: u64 = 500,
    write_stall_poll_ms: u64 = 100,
}

impl RecoverySettings {
    pub fn validate(&self) -> Result<(), String> {
        let nonzero = [
            ("reset_attempts", u64::from(self.reset_attempts)),
            ("max_reopen_attempts", u64::from(self.max_reopen_attempts)),
            ("write_stall_timeout_ms", self.write_stall_timeout_ms),
            ("write_stall_poll_ms", self.write_stall_poll_ms),
        ];
        match nonzero.iter().find(|(_, value)| *value == 0) {
            Some((name, _)) => Err(format!("recovery.{name} must be at least 1")),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_carries_only_changed_fields_and_round_trips() {
        let tuned = RecoverySettings {
            reset_settle_ms: 250,
            enabled: false,
            ..RecoverySettings::default()
        };

        let diff = tuned
            .diff_from(&RecoverySettings::default())
            .expect("changed fields produce a diff");
        assert_eq!(diff.reset_settle_ms, Some(250));
        assert_eq!(diff.enabled, Some(false));
        assert_eq!(diff.quick_retries, None);

        let mut merged = RecoverySettings::default();
        merged.apply_overrides(diff);
        assert_eq!(merged, tuned);
    }

    #[test]
    fn defaults_produce_no_diff() {
        let defaults = RecoverySettings::default();
        assert_eq!(defaults.diff_from(&defaults), None);
    }

    #[test]
    fn zero_poll_interval_is_rejected() {
        let settings = RecoverySettings {
            write_stall_poll_ms: 0,
            ..RecoverySettings::default()
        };
        assert_eq!(
            settings.validate(),
            Err("recovery.write_stall_poll_ms must be at least 1".to_string())
        );
    }
}

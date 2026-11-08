//! Advanced preferences: USB auto-recovery switch and timing. Compiled only
//! with the `recovery-tuning` feature.

use iced::widget::{checkbox, column, row, text, text_input};
use iced::{Element, Length};
use settings::RecoverySettings;
use settings::recovery::PartialRecoverySettings;

use crate::app::State;
use crate::message::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryField {
    QuickRetries,
    QuickRetryDelayMs,
    ResetAttempts,
    ResetSettleMs,
    BackoffInitialMs,
    BackoffCapMs,
    MaxReopenAttempts,
    MaxConsecutiveRecoveries,
    WriteStallTimeoutMs,
    WriteStallPollMs,
}

impl RecoveryField {
    const ALL: [Self; 10] = [
        Self::QuickRetries,
        Self::QuickRetryDelayMs,
        Self::ResetAttempts,
        Self::ResetSettleMs,
        Self::BackoffInitialMs,
        Self::BackoffCapMs,
        Self::MaxReopenAttempts,
        Self::MaxConsecutiveRecoveries,
        Self::WriteStallTimeoutMs,
        Self::WriteStallPollMs,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::QuickRetries => "Read retries",
            Self::QuickRetryDelayMs => "Read retry delay (ms)",
            Self::ResetAttempts => "Resets per cycle",
            Self::ResetSettleMs => "Reset settle (ms)",
            Self::BackoffInitialMs => "Backoff start (ms)",
            Self::BackoffCapMs => "Backoff cap (ms)",
            Self::MaxReopenAttempts => "Reopen cycles",
            Self::MaxConsecutiveRecoveries => "Recoveries before lost",
            Self::WriteStallTimeoutMs => "Write stall timeout (ms)",
            Self::WriteStallPollMs => "Watchdog poll (ms)",
        }
    }

    fn get(self, s: &RecoverySettings) -> u64 {
        match self {
            Self::QuickRetries => s.quick_retries.into(),
            Self::QuickRetryDelayMs => s.quick_retry_delay_ms,
            Self::ResetAttempts => s.reset_attempts.into(),
            Self::ResetSettleMs => s.reset_settle_ms,
            Self::BackoffInitialMs => s.backoff_initial_ms,
            Self::BackoffCapMs => s.backoff_cap_ms,
            Self::MaxReopenAttempts => s.max_reopen_attempts.into(),
            Self::MaxConsecutiveRecoveries => s.max_consecutive_recoveries.into(),
            Self::WriteStallTimeoutMs => s.write_stall_timeout_ms,
            Self::WriteStallPollMs => s.write_stall_poll_ms,
        }
    }

    /// `None` when `input` is not a value this field can hold.
    fn delta(self, input: &str) -> Option<PartialRecoverySettings> {
        let value: u64 = input.trim().parse().ok()?;
        let count = || u32::try_from(value).ok();
        let mut d = PartialRecoverySettings::default();
        match self {
            Self::QuickRetries => d.quick_retries = Some(count()?),
            Self::QuickRetryDelayMs => d.quick_retry_delay_ms = Some(value),
            Self::ResetAttempts => d.reset_attempts = Some(count()?),
            Self::ResetSettleMs => d.reset_settle_ms = Some(value),
            Self::BackoffInitialMs => d.backoff_initial_ms = Some(value),
            Self::BackoffCapMs => d.backoff_cap_ms = Some(value),
            Self::MaxReopenAttempts => d.max_reopen_attempts = Some(count()?),
            Self::MaxConsecutiveRecoveries => d.max_consecutive_recoveries = Some(count()?),
            Self::WriteStallTimeoutMs => d.write_stall_timeout_ms = Some(value),
            Self::WriteStallPollMs => d.write_stall_poll_ms = Some(value),
        }
        Some(d)
    }
}

#[derive(Debug, Clone)]
pub enum AdvancedMessage {
    ToggleRecovery(bool),
    Input(RecoveryField, String),
    Commit,
}

/// The field being typed and its in-progress text. Edits apply on Enter, or
/// when the user moves on to another field, so a half-typed number never
/// reaches the driver.
pub type RecoveryEdit = Option<(RecoveryField, String)>;

fn delta(recovery: PartialRecoverySettings) -> settings::PartialSettings {
    settings::PartialSettings {
        recovery: Some(recovery),
        ..Default::default()
    }
}

fn commit(state: &mut State) {
    if let Some((field, input)) = state.recovery_edit.take()
        && let Some(recovery) = field.delta(&input)
    {
        state.send_apply(delta(recovery), true);
    }
}

pub fn update(state: &mut State, message: AdvancedMessage) {
    match message {
        AdvancedMessage::ToggleRecovery(on) => state.send_apply(
            delta(PartialRecoverySettings {
                enabled: Some(on),
                ..Default::default()
            }),
            true,
        ),
        AdvancedMessage::Input(field, input) => {
            if state
                .recovery_edit
                .as_ref()
                .is_some_and(|(f, _)| *f != field)
            {
                commit(state);
            }
            state.recovery_edit = Some((field, input));
        }
        AdvancedMessage::Commit => commit(state),
    }
}

pub fn view<'a>(state: &'a State, recovery: &RecoverySettings) -> Element<'a, Message> {
    let field_input = |field: RecoveryField| -> Element<'a, Message> {
        let value = match &state.recovery_edit {
            Some((f, input)) if *f == field => input.clone(),
            _ => field.get(recovery).to_string(),
        };
        row![
            text(field.label()).width(Length::Fill),
            text_input("", &value)
                .on_input(move |s| Message::Advanced(AdvancedMessage::Input(field, s)))
                .on_submit(Message::Advanced(AdvancedMessage::Commit))
                .width(Length::Fixed(80.0)),
        ]
        .spacing(8)
        .align_y(iced::alignment::Vertical::Center)
        .into()
    };

    let mut fields = column![].spacing(6);
    for pair in RecoveryField::ALL.chunks(2) {
        let mut line = row![].spacing(16);
        for &field in pair {
            line = line.push(field_input(field));
        }
        fields = fields.push(line);
    }

    column![
        checkbox(recovery.enabled)
            .label("USB auto-recovery")
            .on_toggle(|on| Message::Advanced(AdvancedMessage::ToggleRecovery(on))),
        fields,
    ]
    .spacing(10)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_field_rejects_values_past_u32() {
        assert_eq!(RecoveryField::ResetAttempts.delta("4294967296"), None);
        assert_eq!(
            RecoveryField::ResetAttempts
                .delta("3")
                .and_then(|d| d.reset_attempts),
            Some(3)
        );
    }

    #[test]
    fn non_numeric_input_produces_no_delta() {
        assert_eq!(RecoveryField::ResetSettleMs.delta("fast"), None);
        assert_eq!(RecoveryField::ResetSettleMs.delta(""), None);
    }

    #[test]
    fn moving_to_another_field_commits_the_previous_edit() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut state = State {
            settings: Some(std::sync::Arc::new(settings::Settings::default())),
            sender: Some(tx),
            ..State::default()
        };

        update(
            &mut state,
            AdvancedMessage::Input(RecoveryField::ResetSettleMs, "250".into()),
        );
        update(
            &mut state,
            AdvancedMessage::Input(RecoveryField::BackoffCapMs, "1".into()),
        );

        let settings = state.settings.as_ref().unwrap();
        assert_eq!(settings.recovery.reset_settle_ms, 250);
        assert_eq!(
            settings.recovery.backoff_cap_ms,
            RecoverySettings::default().backoff_cap_ms,
            "the field still being typed is not applied"
        );
    }
}

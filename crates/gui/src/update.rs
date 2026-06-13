//! Top-level update handler.

use iced::Task;
use std::sync::Arc;

use crate::app::State;
use crate::device::view::control_index_valid;
use crate::message::Message;

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    use protocol::{DriverToGui, GuiToDriver};

    match message {
        Message::Ready(sender) => {
            let _ = sender.send(GuiToDriver::GetSettings);
            let _ = sender.send(GuiToDriver::SubscribeEvents);
            state.sender = Some(sender);
            state.status = "connected".to_string();
        }
        Message::Frame(DriverToGui::Settings(snapshot)) => {
            state.settings = Some(Arc::from(*snapshot));
        }
        Message::Frame(DriverToGui::Ack { result, .. }) => {
            if let Err(message) = result {
                state.status = format!("apply rejected: {message}");
                if let Some(sender) = &state.sender {
                    let _ = sender.send(GuiToDriver::GetSettings);
                }
            }
        }
        Message::Frame(DriverToGui::ControlActuated { .. }) => {}
        Message::Frame(DriverToGui::MidiActivity { .. }) => {}
        Message::Frame(DriverToGui::DeviceConnected(connected)) => {
            state.device_connected = connected;
        }
        Message::Tick => {}
        Message::Disconnected => {
            state.sender = None;
            state.device_connected = false;
            state.status = "disconnected".to_string();
        }
        Message::Error(err) => {
            state.sender = None;
            state.device_connected = false;
            state.status = format!("error: {err}");
        }
        Message::SelectControl(control) => {
            state.selection = vec![control];
        }
        Message::SelectControls(controls) => {
            let filtered: Vec<_> = controls
                .into_iter()
                .filter(|c| control_index_valid(*c))
                .collect();
            // An empty drag (covered nothing) leaves the selection untouched.
            if !filtered.is_empty() {
                state.selection = filtered;
            }
        }
        Message::ToggleControl(control) => {
            if control_index_valid(control) {
                if state.selection.is_empty() {
                    state.selection = vec![control];
                } else if same_control_kind(&state.selection[0], &control) {
                    if let Some(pos) = state.selection.iter().position(|c| *c == control) {
                        state.selection.remove(pos);
                    } else {
                        state.selection.push(control);
                    }
                }
                // Different kind than the current selection: ignored.
            }
        }
        Message::ToggleShowAllLabels(on) => {
            state.show_all_labels = on;
        }
    }
    Task::none()
}

/// Whether two control refs are the same kind (Pad/Button/Encoder/Slider).
/// Selection is mutually exclusive across kinds, so Ctrl+click only toggles
/// within the kind already selected.
fn same_control_kind(a: &protocol::ControlRef, b: &protocol::ControlRef) -> bool {
    use protocol::ControlRef::*;
    matches!(
        (a, b),
        (Pad(_), Pad(_)) | (Button(_), Button(_)) | (Encoder, Encoder) | (Slider, Slider)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::State;
    use protocol::{DriverToGui, GuiToDriver};

    /// A connected `State` with a snapshot already adopted, plus the channel the
    /// driver would read outgoing frames from.
    fn seeded() -> (State, std::sync::mpsc::Receiver<GuiToDriver>) {
        let mut state = State::default();
        let (tx, rx) = std::sync::mpsc::channel();
        state.sender = Some(tx);
        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Settings(Box::default())),
        );
        (state, rx)
    }

    #[test]
    fn ctrl_click_toggles_same_kind_membership() {
        use protocol::ControlRef;
        let (mut state, _rx) = seeded();
        state.selection = vec![ControlRef::Button(1)];
        let _ = update(&mut state, Message::ToggleControl(ControlRef::Button(2)));
        assert_eq!(
            state.selection,
            vec![ControlRef::Button(1), ControlRef::Button(2)]
        );
        let _ = update(&mut state, Message::ToggleControl(ControlRef::Button(1)));
        assert_eq!(state.selection, vec![ControlRef::Button(2)]);
    }

    #[test]
    fn ctrl_click_different_kind_is_ignored() {
        use protocol::ControlRef;
        let (mut state, _rx) = seeded();
        state.selection = vec![ControlRef::Button(1)];
        let _ = update(&mut state, Message::ToggleControl(ControlRef::Pad(12)));
        assert_eq!(state.selection, vec![ControlRef::Button(1)]);
    }

    #[test]
    fn ctrl_click_into_empty_selects_the_control() {
        use protocol::ControlRef;
        let (mut state, _rx) = seeded();
        assert!(state.selection.is_empty());
        let _ = update(&mut state, Message::ToggleControl(ControlRef::Pad(12)));
        assert_eq!(state.selection, vec![ControlRef::Pad(12)]);
    }

    #[test]
    fn empty_drag_keeps_current_selection() {
        use protocol::ControlRef;
        let (mut state, _rx) = seeded();
        state.selection = vec![ControlRef::Pad(12)];
        let _ = update(&mut state, Message::SelectControls(vec![]));
        assert_eq!(state.selection, vec![ControlRef::Pad(12)]);
    }
}

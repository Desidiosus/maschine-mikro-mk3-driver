//! Top-level update handler.

use iced::Task;
use std::sync::Arc;

use crate::app::State;
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
    }
    Task::none()
}

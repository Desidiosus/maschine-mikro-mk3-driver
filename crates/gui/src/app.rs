use std::sync::Arc;

use iced::widget::{column, container, text};
use iced::{Element, Length, Subscription, Task};
use protocol::GuiToDriver;
use settings::Settings;

use crate::message::Message;

#[derive(Default)]
pub struct State {
    pub(crate) status: String,
    /// Shared so the per-frame device overlay clones a pointer, not the whole
    /// nested settings tree.
    pub(crate) settings: Option<Arc<Settings>>,
    pub(crate) sender: Option<std::sync::mpsc::Sender<GuiToDriver>>,
    pub(crate) device_connected: bool,
}

impl State {
    pub fn new() -> Self {
        Self {
            status: "connecting…".to_string(),
            ..Self::default()
        }
    }

    pub fn title(&self) -> String {
        "Maschine Mikro MK3 — Configuration".to_string()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        crate::update::update(self, message)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let presence = if self.device_connected {
            "device connected"
        } else {
            "no device"
        };
        let loaded = if self.settings.is_some() {
            "settings loaded"
        } else {
            "waiting for settings…"
        };
        container(column![text(self.status.clone()), text(presence), text(loaded)].spacing(8))
            .padding(16)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(crate::io::subscription::driver_connection),
            iced::time::every(std::time::Duration::from_millis(120)).map(|_| Message::Tick),
        ])
    }
}

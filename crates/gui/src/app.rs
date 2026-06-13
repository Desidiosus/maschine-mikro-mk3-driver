use std::sync::Arc;

use iced::widget::{checkbox, column, container, row, text};
use iced::{Element, Length, Subscription, Task};
use protocol::{ControlRef, GuiToDriver};
use settings::Settings;

use crate::device::hotspots::Device;
use crate::device::view::device_view;
use crate::message::Message;

pub struct State {
    pub(crate) status: String,
    /// Shared so the per-frame device overlay clones a pointer, not the whole
    /// nested settings tree.
    pub(crate) settings: Option<Arc<Settings>>,
    pub(crate) sender: Option<std::sync::mpsc::Sender<GuiToDriver>>,
    pub(crate) device_connected: bool,
    pub(crate) device: std::sync::Arc<Device>,
    pub(crate) selection: Vec<ControlRef>,
    pub(crate) show_all_labels: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            status: String::new(),
            settings: None,
            sender: None,
            device_connected: false,
            device: std::sync::Arc::new(Device::load()),
            selection: Vec::new(),
            show_all_labels: false,
        }
    }
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
        let header = row![text(self.status.clone()), text(presence), text(loaded)].spacing(16);
        let device_pane = container(
            column![
                container(
                    checkbox(self.show_all_labels)
                        .label("Show all labels")
                        .on_toggle(Message::ToggleShowAllLabels),
                )
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Right),
                container(device_view(self)).height(Length::Fill),
            ]
            .spacing(6),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced::alignment::Vertical::Top)
        .padding(8);
        column![header, device_pane].spacing(4).into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(crate::io::subscription::driver_connection),
            iced::time::every(std::time::Duration::from_millis(120)).map(|_| Message::Tick),
        ])
    }
}

use iced::widget::{container, row, text};
use iced::{Element, Length};

use crate::message::Message;

/// A labelled MIDI activity LED: a small dot that lights green when `on`.
pub fn activity_led<'a>(label: &str, on: bool) -> Element<'a, Message> {
    let color = if on {
        iced::Color::from_rgb(0.447, 0.808, 0.176)
    } else {
        iced::Color::from_rgb(0.25, 0.25, 0.28)
    };
    let dot = container(text(""))
        .width(Length::Fixed(10.0))
        .height(Length::Fixed(10.0))
        .style(move |_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(color)),
            border: iced::Border {
                radius: 5.0.into(),
                ..Default::default()
            },
            ..container::Style::default()
        });
    row![text(label.to_string()), dot]
        .spacing(6)
        .align_y(iced::alignment::Vertical::Center)
        .into()
}

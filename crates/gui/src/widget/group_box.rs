use iced::widget::container;
use iced::{Background, Border, Color, Element, Length, Theme};

use crate::message::Message;

/// A grouping card: faint background + border + padding, used to visually
/// separate logical groups (e.g. action params vs. LED params).
pub(crate) fn group_box<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content.into())
        .width(Length::Fill)
        .padding(12)
        .style(|_t: &Theme| container::Style {
            background: Some(Background::Color(Color::from_rgb(0.14, 0.14, 0.17))),
            border: Border {
                color: Color::from_rgb(0.28, 0.28, 0.32),
                width: 1.0,
                radius: 5.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

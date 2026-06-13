use iced::widget::{button, column, container, text};
use iced::{Background, Border, Color, Element, Length, Theme};

use crate::message::Message;

/// A single tab-styled button: active tab uses the panel background + white text
/// with top-rounded corners; inactive tabs are darker and muted. The caller
/// supplies `on_press` so this function is decoupled from any specific tab enum.
pub(crate) fn tab_button<'a>(
    label: &str,
    active: bool,
    on_press: Message,
) -> iced::widget::Button<'a, Message> {
    button(text(label.to_string()).size(13))
        .padding([6, 18])
        .on_press(on_press)
        .style(move |_t: &Theme, _s| iced::widget::button::Style {
            background: Some(Background::Color(if active {
                Color::from_rgb(0.20, 0.20, 0.25)
            } else {
                Color::from_rgb(0.12, 0.12, 0.15)
            })),
            text_color: if active {
                Color::WHITE
            } else {
                Color::from_rgb(0.55, 0.55, 0.6)
            },
            border: Border {
                color: Color::from_rgb(0.30, 0.30, 0.34),
                width: 1.0,
                radius: iced::border::Radius {
                    top_left: 5.0,
                    top_right: 5.0,
                    bottom_right: 0.0,
                    bottom_left: 0.0,
                },
            },
            ..Default::default()
        })
}

/// A 1px horizontal divider line.
pub(crate) fn divider<'a>() -> Element<'a, Message> {
    container(column![])
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &Theme| container::Style {
            background: Some(Background::Color(Color::from_rgb(0.30, 0.30, 0.34))),
            ..container::Style::default()
        })
        .into()
}

/// Wrap a row of tabs with a bottom rule so it reads as a tab bar.
pub(crate) fn tab_bar<'a>(tabs: iced::widget::Row<'a, Message>) -> Element<'a, Message> {
    column![tabs.spacing(2), divider()].spacing(0).into()
}

use iced::widget::{button, container, row, text};
use iced::{Element, Length};

use crate::app::State;
use crate::message::Message;
use crate::widget::activity_led::activity_led;
use crate::widget::icon::{SETTINGS_SVG, USB_OFF_SVG, USB_SVG, svg_icon};

/// How long an activity LED stays lit after the last MIDI event. Also gates the
/// redraw timer (`app::State::subscription`) so an idle GUI does no periodic work.
pub(crate) const ACTIVITY_WINDOW_MS: u128 = 180;

pub(crate) fn top_bar(state: &State) -> Element<'_, Message> {
    let usb_icon = if state.device_connected {
        svg_icon(USB_SVG, iced::Color::from_rgb(0.447, 0.808, 0.176), 22.0)
    } else {
        svg_icon(USB_OFF_SVG, iced::Color::from_rgb(0.886, 0.0, 0.0), 22.0)
    };
    let settings_btn = button(svg_icon(
        SETTINGS_SVG,
        iced::Color::from_rgb(0.85, 0.85, 0.88),
        20.0,
    ))
    .on_press(Message::TogglePrefs)
    .padding(6)
    .style(|_t: &iced::Theme, status| {
        let bg = match status {
            button::Status::Hovered | button::Status::Pressed => {
                iced::Color::from_rgb(0.24, 0.24, 0.29)
            }
            _ => iced::Color::from_rgb(0.17, 0.17, 0.21),
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::Border {
                color: iced::Color::from_rgb(0.35, 0.35, 0.40),
                width: 1.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        }
    });
    let now = std::time::Instant::now();
    let lit = |t: Option<std::time::Instant>| {
        t.is_some_and(|t| now.duration_since(t).as_millis() < ACTIVITY_WINDOW_MS)
    };
    container(
        row![
            usb_icon,
            activity_led("In", lit(state.last_in)),
            activity_led("Out", lit(state.last_out)),
            text("").width(Length::Fill),
            settings_btn,
        ]
        .spacing(16)
        .align_y(iced::alignment::Vertical::Center)
        .padding([6, 10]),
    )
    .width(Length::Fill)
    .style(|_t: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(iced::Color::from_rgb(
            0.10, 0.10, 0.12,
        ))),
        border: iced::Border {
            color: iced::Color::from_rgb(0.0, 0.0, 0.0),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .into()
}

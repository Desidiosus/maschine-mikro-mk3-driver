//! Preferences overlay view functions.

use iced::widget::{checkbox, column, pick_list, row, slider, text};
use iced::{Element, Length};
use settings::{BacklightBrightness, PadVelocityCurve};

use crate::app::State;
use crate::message::Message;

/// A top-level preferences section header: medium-weight label over a full-width rule.
pub(crate) fn pref_header<'a>(title: &str) -> Element<'a, Message> {
    use iced::widget::container;
    let label = text(title.to_string())
        .size(15)
        .font(iced::Font {
            weight: iced::font::Weight::Medium,
            ..iced::Font::DEFAULT
        })
        .color(iced::Color::from_rgb(0.88, 0.88, 0.92));
    let underline = container(text(""))
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(
                0.40, 0.40, 0.46,
            ))),
            ..container::Style::default()
        });
    column![label, underline]
        .spacing(5)
        .width(Length::Fill)
        .into()
}

/// A subsection header inside a group (e.g. Pads / Display / LEDs under Device settings).
fn sub_header<'a>(title: &str) -> Element<'a, Message> {
    text(title.to_string())
        .size(13)
        .font(iced::Font {
            weight: iced::font::Weight::Medium,
            ..iced::Font::DEFAULT
        })
        .color(iced::Color::from_rgb(0.60, 0.60, 0.66))
        .into()
}

pub fn prefs_overlay(state: &State) -> Element<'_, Message> {
    use iced::widget::{button, container, mouse_area};
    use iced::{Background, Border, Color, Theme};

    let Some(settings) = state.settings.as_ref() else {
        return iced::widget::column![].into();
    };
    let h = &settings.hardware;

    fn labeled<'a>(label: String, control: Element<'a, Message>) -> iced::widget::Row<'a, Message> {
        row![text(label).width(Length::Fixed(160.0)), control]
            .spacing(12)
            .align_y(iced::alignment::Vertical::Center)
    }

    let editor = column![labeled(
        "Touch Select".to_string(),
        checkbox(state.touch_select)
            .on_toggle(Message::ToggleTouchSelect)
            .into(),
    )]
    .spacing(10);

    let device = column![
        sub_header("Pads"),
        labeled(
            "Pad sensitivity".to_string(),
            slider(0..=100, h.pad_sensitivity, Message::PreviewPadSensitivity)
                .on_release(Message::SetPadSensitivity)
                .into(),
        ),
        labeled(
            "Pad velocity curve".to_string(),
            pick_list(
                &PadVelocityCurve::ALL[..],
                Some(h.pad_velocity_curve),
                Message::SetVelocityCurve
            )
            .into(),
        ),
        sub_header("Display"),
        labeled(
            "Display contrast".to_string(),
            slider(0..=100, h.display_contrast, Message::PreviewDisplayContrast)
                .on_release(Message::SetDisplayContrast)
                .into(),
        ),
        sub_header("LEDs"),
        labeled(
            "Button backlight".to_string(),
            checkbox(h.backlight_buttons)
                .on_toggle(Message::SetBacklightButtons)
                .into(),
        ),
        labeled(
            "Backlight brightness".to_string(),
            pick_list(
                &BacklightBrightness::ALL[..],
                Some(h.backlight_brightness),
                Message::SetBacklightBrightness
            )
            .into(),
        ),
    ]
    .spacing(10);

    let panel = container(
        column![
            pref_header("Editor settings"),
            crate::widget::group_box::group_box(editor),
            pref_header("Device settings"),
            crate::widget::group_box::group_box(device),
            container(button("Close").on_press(Message::TogglePrefs))
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Right),
        ]
        .spacing(14)
        .padding(20),
    )
    .width(Length::Fixed(460.0))
    .height(Length::Shrink)
    .style(|_t: &Theme| container::Style {
        background: Some(Background::Color(Color::from_rgb(0.13, 0.13, 0.16))),
        border: Border {
            color: Color::BLACK,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    });

    // Swallow clicks inside the panel so they don't reach the backdrop.
    let panel = mouse_area(panel).on_press(Message::Ignore);
    // Dimming backdrop that captures the click: closes on an outside press and
    // blocks the device diagram beneath the overlay.
    mouse_area(
        container(panel)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_t: &Theme| container::Style {
                background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.4))),
                ..container::Style::default()
            }),
    )
    .on_press(Message::TogglePrefs)
    .into()
}

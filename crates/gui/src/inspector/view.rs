use iced::widget::{column, container, row, scrollable, text};
use iced::{Element, Length};

use crate::app::State;
use crate::message::Message;

pub(crate) fn inspector(state: &State) -> Element<'_, Message> {
    let assign_tab = container(text("Assign").size(13).color(iced::Color::WHITE))
        .padding([6, 18])
        .style(|_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(
                0.20, 0.20, 0.25,
            ))),
            border: iced::Border {
                color: iced::Color::from_rgb(0.30, 0.30, 0.34),
                width: 1.0,
                radius: iced::border::Radius {
                    top_left: 5.0,
                    top_right: 5.0,
                    bottom_right: 0.0,
                    bottom_left: 0.0,
                },
            },
            ..Default::default()
        });
    let assign_tab_header = column![row![assign_tab], crate::widget::tabs::divider()]
        .spacing(0)
        .width(Length::Fill);
    let inspector_body = container(
        column![
            assign_tab_header,
            crate::inspector::assign::view::assignment_body(state)
        ]
        .spacing(8),
    )
    .padding(12);

    container(scrollable(inspector_body))
        .width(Length::Fixed(340.0))
        .height(Length::Fill)
        .padding(8)
        .style(|_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(
                0.12, 0.12, 0.14,
            ))),
            border: iced::Border {
                color: iced::Color::from_rgb(0.0, 0.0, 0.0),
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        })
        .into()
}

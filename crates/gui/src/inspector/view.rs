use iced::widget::{column, container, scrollable};
use iced::{Element, Length};

use crate::app::State;
use crate::message::{InspectorTab, Message};
use crate::widget::tabs::{tab_bar, tab_button};

pub(crate) fn inspector(state: &State) -> Element<'_, Message> {
    let tabs = iced::widget::Row::with_children(vec![
        tab_button(
            "Pages",
            state.inspector_tab == InspectorTab::Pages,
            Message::SetInspectorTab(InspectorTab::Pages),
        )
        .into(),
        tab_button(
            "Assign",
            state.inspector_tab == InspectorTab::Assign,
            Message::SetInspectorTab(InspectorTab::Assign),
        )
        .into(),
    ]);
    let header = tab_bar(tabs);

    let body: Element<'_, Message> = match state.inspector_tab {
        InspectorTab::Assign => crate::inspector::assign::view::assignment_body(state),
        InspectorTab::Pages => crate::inspector::pages::view::pages_body(state),
    };

    let inspector_body = container(column![header, body].spacing(8)).padding(12);

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

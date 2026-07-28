use iced::widget::{column, text};
use iced::{Element, Length};

use crate::app::State;
use crate::message::Message;

/// The Pages tab body. Task 3 fills in the list, icons, and color pickers.
pub(crate) fn pages_body(state: &State) -> Element<'_, Message> {
    let Some(settings) = &state.settings else {
        return text("Waiting for device settings…").into();
    };
    if !settings.pad_paging.enabled {
        // Disabled: body is (near) empty per the reference screenshots; Task 3
        // adds the enable toggle to the header.
        return column![text("Pad paging is disabled.")]
            .width(Length::Fill)
            .into();
    }
    column![text(format!("{} page(s)", settings.pad_paging.pages.len()))]
        .width(Length::Fill)
        .into()
}

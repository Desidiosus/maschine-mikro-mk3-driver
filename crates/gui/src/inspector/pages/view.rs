use iced::widget::{Space, button, column, container, mouse_area, row, text, text_input};
use iced::{Background, Border, Color, Element, Length, Theme};

use crate::app::State;
use crate::inspector::assign::view::labeled_pick_list;
use crate::message::Message;
use crate::widget::group_box::group_box;
use crate::widget::icon::{ADD_SVG, DELETE_SVG, DUPLICATE_SVG, POWER_SVG, svg_icon};

/// Width of the row's leading position-letter cell. Fixed and narrow so the
/// remaining `Length::Fill` name field takes whatever the panel doesn't need
/// for chrome.
const SLOT_WIDTH: f32 = 16.0;
/// Gap between the elements in a row (the position letter and the name).
const ROW_SPACING: f32 = 10.0;

/// Position label for a row: "A" for slot 0 through "P" for slot 15,
/// recomputed on reorder (names, by contrast, are stable). Numeric only in
/// the defensive out-of-range case.
pub(crate) fn slot_letter(i: usize) -> String {
    if i < 26 {
        ((b'A' + i as u8) as char).to_string()
    } else {
        (i + 1).to_string()
    }
}

fn icon_button<'a>(bytes: &'static [u8], msg: Option<Message>) -> Element<'a, Message> {
    let mut b = button(svg_icon(bytes, Color::from_rgb(0.85, 0.85, 0.88), 20.0)).padding(8);
    if let Some(m) = msg {
        b = b.on_press(m);
    }
    b.into()
}

/// Emphasized (suggested-action) button style: the accent blue already used
/// for the selected row background in this panel. Reserved for the safe
/// default action in the delete-confirmation dialog (Cancel), per GNOME HIG:
/// the non-destructive default is emphasized, not the destructive one.
fn emphasized_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => Color::from_rgb(0.20, 0.64, 0.92),
        _ => Color::from_rgb(0.12, 0.56, 0.84),
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::WHITE,
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// De-emphasized destructive-action button style: muted red, brighter on
/// hover. Reserved for Delete, which sits on the left of the dialog per
/// GNOME HIG so it's never where a reflexive "confirm" click lands.
fn destructive_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => Color::from_rgb(0.68, 0.28, 0.28),
        _ => Color::from_rgb(0.55, 0.22, 0.22),
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::WHITE,
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// The row's background: solid accent-blue when selected, grey when not.
///
/// The unselected grey must sit clearly above the group-box card behind the
/// list (`widget::group_box`, `rgb(0.14, 0.14, 0.17)`) so rows read as
/// distinct chips rather than blending into the panel.
fn row_background_style(selected: bool) -> impl Fn(&Theme) -> container::Style {
    move |_theme: &Theme| {
        let background = if selected {
            // Deep enough that pure-white row text clears ~4.5:1 contrast; a
            // lighter, more saturated blue washed the name out when selected.
            Color::from_rgb(0.10, 0.20, 0.50)
        } else {
            Color::from_rgb(0.23, 0.23, 0.27)
        };
        container::Style {
            background: Some(Background::Color(background)),
            ..container::Style::default()
        }
    }
}

/// Resolved (named-value, default-placeholder) text colours for a page name
/// label. On the selected (deep-blue) row the value must be pure white; and
/// for an unnamed legacy page the placeholder IS the visible label, so it
/// can't use the usual dim hint grey — that made every default-named page
/// hard to read. Shared by the plain-text display (the default, non-editing
/// look) and by the inline rename field's placeholder.
fn name_colors(selected: bool) -> (Color, Color) {
    if selected {
        (Color::WHITE, Color::from_rgb(0.90, 0.93, 0.99))
    } else {
        (
            Color::from_rgb(0.92, 0.92, 0.95),
            Color::from_rgb(0.78, 0.78, 0.82),
        )
    }
}

/// Style for the page-name `text_input`, shown only while its row is being
/// renamed (`State::editing_page_name == Some(i)`). Unlike plain row text,
/// this must be unmistakably a field: a visible accent border and a
/// background darker than either row state, always on (not just while
/// focused) since the mere presence of the `text_input` already means the
/// user opted in by double-clicking the row.
fn editing_name_input_style(
    _theme: &Theme,
    _status: iced::widget::text_input::Status,
) -> iced::widget::text_input::Style {
    iced::widget::text_input::Style {
        background: Background::Color(Color::from_rgb(0.08, 0.08, 0.11)),
        border: Border {
            color: Color::from_rgb(0.40, 0.55, 0.85),
            width: 1.0,
            radius: 3.0.into(),
        },
        icon: Color::from_rgb(0.85, 0.85, 0.88),
        placeholder: Color::from_rgb(0.70, 0.70, 0.75),
        value: Color::from_rgb(0.95, 0.95, 0.97),
        selection: Color::from_rgb(0.25, 0.45, 0.75),
    }
}

/// Subtle icon-button chrome for the paging enable toggle: a dark rounded
/// background that lightens on hover, not iced's default solid-blue button
/// fill, so it reads as a toggleable icon affordance rather than a primary
/// action. Mirrors the settings-gear button in `crate::shell::view::top_bar`.
fn subtle_toggle_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => Color::from_rgb(0.24, 0.24, 0.29),
        _ => Color::from_rgb(0.17, 0.17, 0.21),
    };
    button::Style {
        background: Some(Background::Color(background)),
        border: Border {
            color: Color::from_rgb(0.35, 0.35, 0.40),
            width: 1.0,
            radius: 6.0.into(),
        },
        ..button::Style::default()
    }
}

/// The stable per-row id for the page-name `text_input`, shared between the
/// row that builds it (only while editing) and `Message::BeginRenamePage`'s
/// focus request, so both sides always agree on which widget to focus.
pub(crate) fn page_name_input_id(i: usize) -> String {
    format!("page-name-{i}")
}

/// The Pages tab body: enable toggle, a flat page-name list,
/// Add/Duplicate/Delete actions below it, and a panel-level default color.
pub(crate) fn pages_body(state: &State) -> Element<'_, Message> {
    let Some(settings) = &state.settings else {
        return text("Waiting for device settings…").into();
    };
    let pp = &settings.pad_paging;

    // Header: title + a power-icon button that toggles paging (top-right).
    let toggle = button(svg_icon(
        POWER_SVG,
        if pp.enabled {
            Color::from_rgb(0.45, 0.90, 0.50)
        } else {
            Color::from_rgb(0.5, 0.5, 0.55)
        },
        18.0,
    ))
    .on_press(Message::SetPagingEnabled(!pp.enabled))
    .padding(6)
    .style(subtle_toggle_button_style);

    if !pp.enabled {
        let header = row![
            text("Pad Pages")
                .size(14)
                .color(Color::from_rgb(0.55, 0.55, 0.60)),
            Space::new().width(Length::Fill),
            toggle,
        ]
        .spacing(8)
        .align_y(iced::alignment::Vertical::Center);
        return group_box(column![header].spacing(10));
    }

    let header = row![
        text("Pad Pages").size(14),
        Space::new().width(Length::Fill),
        toggle,
    ]
    .spacing(8)
    .align_y(iced::alignment::Vertical::Center);

    let mut list = column![];
    for (i, page) in pp.pages.iter().enumerate() {
        let active = i == pp.active;
        let editing = state.editing_page_name == Some(i);
        let value = page.name.clone().unwrap_or_default();

        // Empty value + a fresh default-name placeholder → clearing the name
        // shows what an empty commit would assign as a hint; typing sets the
        // name, clearing it back to empty resets to None. No trim here:
        // trimming every keystroke would swallow spaces before a trailing
        // word is typed; that happens on commit.
        //
        // Only the row being renamed gets the `text_input`; every other row
        // shows plain `text` — not directly editable, and not styled to look
        // editable — until its own row is double-clicked.
        let name_el: Element<'_, Message> = if editing {
            let placeholder = crate::app::page_ops::next_page_name(pp);
            text_input(&placeholder, &value)
                .id(page_name_input_id(i))
                .on_input(move |s| Message::SetPageName(i, s))
                .on_submit(Message::CommitPageName(i))
                .style(editing_name_input_style)
                .width(Length::Fill)
                .into()
        } else {
            let (value_c, placeholder_c) = name_colors(active);
            let color = if page.name.is_some() {
                value_c
            } else {
                placeholder_c
            };
            // Names are concrete strings assigned at creation now, never
            // derived from position — a `None` here would only mean a
            // theoretically stale/legacy page, so fall back to blank rather
            // than a slot number that could be wrong after a reorder.
            let label = page.name.clone().unwrap_or_default();
            text(label).color(color).width(Length::Fill).into()
        };

        let slot_color = if active {
            Color::WHITE
        } else {
            Color::from_rgb(0.85, 0.85, 0.88)
        };
        let slot = text(slot_letter(i))
            .width(Length::Fixed(SLOT_WIDTH))
            .color(slot_color);

        let row_content = row![slot, name_el]
            .spacing(ROW_SPACING)
            .align_y(iced::alignment::Vertical::Center);

        let row_widget = container(row_content)
            .width(Length::Fill)
            .padding([8.0, 8.0])
            .style(row_background_style(active));

        // A press anywhere in the row selects that page; a double-click opens
        // inline rename. While renaming, the row's `text_input` captures its
        // own presses, so editing never re-selects the row.
        let row_el: Element<'_, Message> = mouse_area(row_widget)
            .on_press(Message::SelectPage(i))
            .on_double_click(Message::BeginRenamePage(i))
            .into();
        list = list.push(row_el);
    }
    let list = list.spacing(2);

    // Add/Duplicate/Delete sit below the list (not in the rows, which
    // overflowed the panel once they carried a color picker and two icon
    // buttons each). They act on the currently active page, except Add,
    // which always appends. Delete opens a confirmation dialog rather than
    // deleting immediately (see `delete_page_overlay`).
    let actions = row![
        icon_button(
            ADD_SVG,
            (pp.pages.len() < settings::MAX_PAGES).then_some(Message::AddPage),
        ),
        icon_button(
            DUPLICATE_SVG,
            (pp.pages.len() < settings::MAX_PAGES).then_some(Message::DuplicatePage(pp.active)),
        ),
        icon_button(
            DELETE_SVG,
            (pp.pages.len() > settings::MIN_PAGES).then_some(Message::RequestDeletePage(pp.active)),
        ),
    ]
    .spacing(6);

    let default_color = labeled_pick_list(
        "Default color",
        &settings::PadColors::ALL[1..],
        Some(pp.default_page_color),
        Message::SetDefaultPageColor,
    );

    group_box(column![header, list, actions, default_color].spacing(10))
}

/// The Delete-page confirmation modal, shown while `State::confirm_delete_page`
/// is set. Mirrors `prefs::view::prefs_overlay`'s scrim + centered card
/// pattern: a full-window dimming `mouse_area` closes the dialog on an
/// outside press, and a `mouse_area` around the card itself swallows clicks
/// so they don't reach the scrim.
pub(crate) fn delete_page_overlay(state: &State) -> Element<'_, Message> {
    let (Some(index), Some(settings)) = (state.confirm_delete_page, state.settings.as_ref()) else {
        return column![].into();
    };
    let page_label = settings
        .pad_paging
        .pages
        .get(index)
        .map(|p| {
            p.name
                .clone()
                .unwrap_or_else(|| format!("Pad Page {}", slot_letter(index)))
        })
        .unwrap_or_else(|| format!("Pad Page {}", slot_letter(index)));

    let body = column![
        text("Delete page?").size(16),
        text(format!(
            "This will delete \"{page_label}\" and its pad mappings."
        ))
        .size(13)
        .color(Color::from_rgb(0.75, 0.75, 0.80)),
        row![
            Space::new().width(Length::Fill),
            // Delete (destructive) on the left, Cancel (safe default,
            // emphasized) on the right — GNOME HIG convention, and the
            // reverse of what a reflexive double-click would confirm.
            button("Delete")
                .on_press(Message::ConfirmDeletePage)
                .padding([6.0, 14.0])
                .style(destructive_button_style),
            button("Cancel")
                .on_press(Message::CancelDeletePage)
                .padding([6.0, 14.0])
                .style(emphasized_button_style),
        ]
        .spacing(8),
    ]
    .spacing(14)
    .padding(20);

    let panel = container(body)
        .width(Length::Fixed(300.0))
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
    .on_press(Message::CancelDeletePage)
    .into()
}

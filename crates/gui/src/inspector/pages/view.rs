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

/// The row's background: solid accent-blue when selected, grey when not, and
/// dimmed (ghosted) while its own page is the one being dragged — checked in
/// that order, since a drag in progress on the active row should still read
/// as "picked up", not "selected".
///
/// The unselected grey must sit clearly above the group-box card behind the
/// list (`widget::group_box`, `rgb(0.14, 0.14, 0.17)`) so rows read as
/// distinct chips rather than blending into the panel; the ghosted colour is
/// opaque and distinctly darker than both for the same reason — a translucent
/// ghost composited invisibly against these close-together darks.
fn row_background_style(
    selected: bool,
    dragging: bool,
    hovered: bool,
) -> impl Fn(&Theme) -> container::Style {
    move |_theme: &Theme| {
        let background = if dragging {
            Color::from_rgb(0.10, 0.10, 0.12)
        } else if selected {
            // Deep enough that pure-white row text clears ~4.5:1 contrast; a
            // lighter, more saturated blue washed the name out when selected.
            Color::from_rgb(0.10, 0.20, 0.50)
        } else if hovered {
            Color::from_rgb(0.29, 0.29, 0.34)
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
/// user opted in by double-clicking the row. Text still fades out while its
/// row is the one being dragged, matching the rest of the row.
fn editing_name_input_style(
    dragging: bool,
) -> impl Fn(&Theme, iced::widget::text_input::Status) -> iced::widget::text_input::Style {
    move |_theme: &Theme, _status: iced::widget::text_input::Status| {
        let alpha = if dragging { 0.35 } else { 1.0 };
        iced::widget::text_input::Style {
            background: Background::Color(Color::from_rgb(0.08, 0.08, 0.11)),
            border: Border {
                color: Color::from_rgb(0.40, 0.55, 0.85),
                width: 1.0,
                radius: 3.0.into(),
            },
            icon: Color::from_rgba(0.85, 0.85, 0.88, alpha),
            placeholder: Color::from_rgba(0.70, 0.70, 0.75, alpha),
            value: Color::from_rgba(0.95, 0.95, 0.97, alpha),
            selection: Color::from_rgba(0.25, 0.45, 0.75, alpha),
        }
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

/// A fixed-height (2px) sliver rendered between every pair of rows (and
/// before the first / after the last), rather than a `container` that's
/// sometimes present and sometimes not. `active` toggles its background
/// between transparent and accent-colored to show the drop target; the
/// widget itself is always there, at the same child index, so the list's
/// shape never changes and the surrounding rows' `text_input` state is never
/// rebuilt because a sibling was conditionally inserted or removed.
fn insertion_gap<'a>(active: bool) -> Element<'a, Message> {
    container(Space::new().height(Length::Fixed(2.0)))
        .width(Length::Fill)
        .height(Length::Fixed(2.0))
        .style(move |_t: &Theme| container::Style {
            background: Some(Background::Color(if active {
                Color::from_rgb(0.30, 0.55, 0.95)
            } else {
                Color::TRANSPARENT
            })),
            ..container::Style::default()
        })
        .into()
}

/// Whether row `i` renders ghosted: it is the drag's origin AND the pointer
/// has crossed into another row. Every press creates drag state — a plain
/// click included — so ghosting on the origin alone made each click flash
/// the drag look for the duration of the press. Gating on movement keeps the
/// ghost, like the insertion line (`drop_gap`), a signal that releasing
/// would move something; it also un-ghosts when the pointer returns to the
/// origin row, where a release is a no-op select.
fn row_is_ghosted(drag: Option<crate::app::PageDrag>, i: usize) -> bool {
    drag.is_some_and(|d| d.from == i && d.over.is_some_and(|over| over != d.from))
}

/// Which gap between the *currently displayed* rows should show the
/// drop-target insertion line, given the raw drag state. Gaps are indexed
/// `0..=len` (gap 0 is above the first row, `len` is below the last).
///
/// `page_ops::reorder` removes `from` and then inserts at `to`, so the
/// dragged page lands at index `to` of the post-removal vector. Relative to
/// the original, pre-removal indices still on screen (the dragged row stays
/// in place, ghosted, during the drag — see `insertion_gap`), that means:
/// after the original row `to` when moving later (`to > from`), or before it
/// when moving earlier (`to < from`). `None` when there's no drag, no row has
/// been entered yet, or the pointer is back over the origin row — a drop
/// there is a no-op (see `Message::PageDragDrop`), not a move, so no gap
/// should light up.
fn drop_gap(from: usize, over: Option<usize>) -> Option<usize> {
    let to = over?;
    if to == from {
        return None;
    }
    Some(if to > from { to + 1 } else { to })
}

/// The Pages tab body: enable toggle, a flat page-name list with drag-reorder,
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

    // Copied out so the loop below can read it without holding a borrow of
    // `state` alongside the per-row `move` closures.
    let page_drag = state.page_drag;
    let dragging_active = page_drag.is_some();
    let target_gap = page_drag.and_then(|d| drop_gap(d.from, d.over));

    let mut list = column![];
    list = list.push(insertion_gap(dragging_active && target_gap == Some(0)));
    for (i, page) in pp.pages.iter().enumerate() {
        let active = i == pp.active;
        let dragging = row_is_ghosted(page_drag, i);
        let editing = state.editing_page_name == Some(i);

        // The open field renders from `State::page_name_text`, not from the
        // stored name: keystrokes are debounced, so the stored name lags behind
        // what has been typed. An empty field shows the name an empty commit
        // would assign as its placeholder.
        //
        // Only the row being renamed gets the `text_input`; every other row
        // shows plain `text` — not directly editable, and not styled to look
        // editable — until its own row is double-clicked.
        let name_el: Element<'_, Message> = if editing {
            let placeholder = pp.next_page_name();
            text_input(&placeholder, &state.page_name_text)
                .id(page_name_input_id(i))
                .on_input(move |s| Message::SetPageName(i, s))
                .on_submit(Message::CommitPageName(i))
                .style(editing_name_input_style(dragging))
                .width(Length::Fill)
                .into()
        } else {
            let (value_c, placeholder_c) = name_colors(active);
            let alpha = if dragging { 0.35 } else { 1.0 };
            let color = Color {
                a: alpha,
                ..(if page.name.is_some() {
                    value_c
                } else {
                    placeholder_c
                })
            };
            // Names are concrete strings assigned at creation now, never
            // derived from position — a `None` here would only mean a
            // theoretically stale/legacy page, so fall back to blank rather
            // than a slot number that could be wrong after a reorder.
            let label = page.name.clone().unwrap_or_default();
            text(label).color(color).width(Length::Fill).into()
        };

        let slot_color = if dragging {
            Color::from_rgba(0.85, 0.85, 0.88, 0.35)
        } else if active {
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

        // Hover highlight is suppressed while a drag is in progress: the
        // insertion line already marks the drop target, and a second
        // highlight under the pointer would compete with it.
        let hovered = !dragging_active && state.hovered_page == Some(i);
        let row_widget = container(row_content)
            .width(Length::Fill)
            .padding([8.0, 8.0])
            .style(row_background_style(active, dragging, hovered));

        // Press anywhere in the row starts a potential drag; releasing
        // without entering another row selects (see `PageDragDrop`).
        // Double-click opens inline rename. While renaming, the row's
        // `text_input` captures its own presses, so editing never starts a
        // drag.
        let row_el: Element<'_, Message> = mouse_area(row_widget)
            .on_press(Message::PageDragStart(i))
            .on_double_click(Message::BeginRenamePage(i))
            .on_enter(Message::PageRowEntered(i))
            .on_exit(Message::PageRowExited(i))
            .into();
        list = list.push(row_el);
        list = list.push(insertion_gap(dragging_active && target_gap == Some(i + 1)));
    }

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

    // A release anywhere over the panel commits the drag (`PageDragDrop`
    // decides reorder vs. select), and `over` keeps whichever row the pointer
    // last entered. Wrapping the whole panel rather than just the rows is what
    // makes dropping a page first or last work: overshooting past the end row
    // into the actions area is the natural way to aim at the end of the list,
    // and the drop still lands on the last row the pointer crossed instead of
    // being cancelled mid-gesture. Leaving the panel entirely still abandons
    // the drag, but that's not the only way `page_drag` could be orphaned — an
    // authoritative settings snapshot or toggling paging off can swap this row
    // list out from under a held drag, so `update` also clears `page_drag` in
    // those paths and `PageDragDrop` re-validates its indices as a last line
    // of defense.
    //
    // The action buttons keep working: iced's `button` publishes and captures a
    // release only when its own press set `is_pressed`, so a drag that ends
    // over one neither adds nor deletes a page, and a real button click is
    // captured before this `mouse_area` ever sees the release.
    let panel = mouse_area(column![header, list, actions, default_color].spacing(10))
        .on_release(Message::PageDragDrop)
        .on_exit(Message::PageDragCancel);

    group_box(panel)
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

#[cfg(test)]
mod row_is_ghosted_tests {
    use super::row_is_ghosted;
    use crate::app::PageDrag;

    #[test]
    fn plain_click_never_ghosts() {
        // A press creates drag state with `over: None`; until the pointer
        // crosses into another row this must not restyle the pressed row.
        let drag = Some(PageDrag {
            from: 1,
            over: None,
        });
        assert!(!row_is_ghosted(drag, 1));
    }

    #[test]
    fn crossing_into_another_row_ghosts_only_the_origin() {
        let drag = Some(PageDrag {
            from: 1,
            over: Some(2),
        });
        assert!(row_is_ghosted(drag, 1));
        assert!(!row_is_ghosted(drag, 2));
    }

    #[test]
    fn returning_to_the_origin_row_unghosts() {
        // Over the origin a release is a no-op select, matching `drop_gap`
        // hiding the insertion line for the same state.
        let drag = Some(PageDrag {
            from: 1,
            over: Some(1),
        });
        assert!(!row_is_ghosted(drag, 1));
    }

    #[test]
    fn no_drag_no_ghost() {
        assert!(!row_is_ghosted(None, 0));
    }
}

#[cfg(test)]
mod drop_gap_tests {
    use super::drop_gap;

    #[test]
    fn no_drag_shows_no_gap() {
        assert_eq!(drop_gap(0, None), None);
    }

    #[test]
    fn hovering_the_origin_row_shows_no_gap() {
        assert_eq!(drop_gap(2, Some(2)), None);
    }

    #[test]
    fn moving_later_highlights_the_gap_after_the_target_row() {
        // [A, B, C, D], dragging A (0) over C (2): A would land between C and D.
        assert_eq!(drop_gap(0, Some(2)), Some(3));
    }

    #[test]
    fn moving_earlier_highlights_the_gap_before_the_target_row() {
        // [A, B, C, D], dragging D (3) over B (1): D would land between A and B.
        assert_eq!(drop_gap(3, Some(1)), Some(1));
    }

    #[test]
    fn adjacent_moves_still_resolve_to_a_single_gap() {
        // Moving one slot later or earlier must not straddle two rows.
        assert_eq!(drop_gap(1, Some(2)), Some(3));
        assert_eq!(drop_gap(2, Some(1)), Some(1));
    }
}

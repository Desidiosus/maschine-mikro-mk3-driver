//! Per-control Assign form widgets and the top-level `assignment_body` builder.

use iced::widget::{checkbox, column, container, pick_list, row, text, text_input};
use iced::{Background, Border, Color, Element, Length, Theme};
use maschine_library::controls::Buttons;
use protocol::ControlRef;
use settings::{
    ButtonPressAction, EncoderTurnAction, PadColors, PadLedMode, PadLedSource, SliderLedMode,
    SliderLedSettings, SliderPositionAction, SliderTouchAction,
};

use crate::app::State;
use crate::inspector::assign::forms::{
    AssignTab, CcType, EncoderModeKind, LedTab, PadHitType, PadPressType, SliderTouchKind,
    cc_type_of_button, cc_type_of_encoder, cc_type_of_position,
};
use crate::inspector::assign::mapping::PadLedColorSlot;
use crate::inspector::assign::multi::{MultiValue, fold};
use crate::inspector::assign::numeric::EditField;
use crate::message::Message;
use crate::widget::group_box::group_box;
use crate::widget::numeric_field::numeric_field;
use crate::widget::tabs::{tab_bar, tab_button};

/// A labeled numeric row: `label  [ value ]`. `active` is the live edit buffer
/// for this field, if it is the one being typed.
pub(crate) fn num_row<'a>(
    label: &str,
    field: EditField,
    value: Option<i8>,
    active: Option<&str>,
) -> Element<'a, Message> {
    row![
        text(label.to_string()).width(Length::Fixed(90.0)),
        numeric_field(field, value, active),
    ]
    .spacing(8)
    .align_y(iced::alignment::Vertical::Center)
    .into()
}

/// Always-visible channel row: `Channel  [ 1..=16 ]`. `value` is the already
/// displayed channel (1..=16), or `None` for an indeterminate multi-selection.
/// Editing writes the per-control channel; there is no global channel.
pub(crate) fn channel_num_row<'a>(
    field: EditField,
    value: Option<i8>,
    active: Option<&str>,
) -> Element<'a, Message> {
    num_row("Channel", field, value, active)
}

/// Framed header: control name + red-bordered assignment box.
pub(crate) fn header<'a>(name: &str, assignment: &str) -> Element<'a, Message> {
    let assignment = container(
        text(assignment.to_string())
            .size(13)
            .color(Color::from_rgb(0.65, 0.65, 0.7)),
    );
    container(column![text(name.to_string()).size(16), assignment].spacing(4))
        .width(Length::Fill)
        .padding(10)
        .style(|_t: &Theme| container::Style {
            background: Some(Background::Color(Color::from_rgb(0.16, 0.16, 0.19))),
            border: Border {
                color: Color::from_rgb(0.30, 0.30, 0.34),
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// `Range` with `From` and `To` stacked on separate lines (encoder Absolute), so
/// the two numeric boxes fit the narrow inspector instead of overflowing one row.
pub(crate) fn range_row<'a>(
    from: u8,
    from_active: Option<&str>,
    to: u8,
    to_active: Option<&str>,
) -> Element<'a, Message> {
    row![
        text("Range").width(Length::Fixed(90.0)),
        column![
            row![
                text("From").width(Length::Fixed(40.0)),
                numeric_field(EditField::EncoderLo, Some(from as i8), from_active),
            ]
            .spacing(8)
            .align_y(iced::alignment::Vertical::Center),
            row![
                text("To").width(Length::Fixed(40.0)),
                numeric_field(EditField::EncoderHi, Some(to as i8), to_active),
            ]
            .spacing(8)
            .align_y(iced::alignment::Vertical::Center),
        ]
        .spacing(6),
    ]
    .spacing(8)
    .align_y(iced::alignment::Vertical::Top)
    .into()
}

/// A sub-action tab strip: one `SetAssignTab` button per `(label, tab)` pair.
pub(crate) fn tab_strip<'a>(tabs: &[(&str, AssignTab)], active: AssignTab) -> Element<'a, Message> {
    tab_bar(iced::widget::Row::with_children(tabs.iter().map(
        |&(label, t)| tab_button(label, active == t, Message::SetAssignTab(t)).into(),
    )))
}

/// Encoder Assign form with Turn | Push | Touch tabs.
pub fn encoder_form<'a>(
    settings: &settings::Settings,
    assignment: &str,
    tab: AssignTab,
    active: &dyn Fn(EditField) -> Option<String>,
) -> Element<'a, Message> {
    let strip = tab_strip(
        &[
            ("Turn", AssignTab::A),
            ("Push", AssignTab::B),
            ("Touch", AssignTab::C),
        ],
        tab,
    );
    let body: Element<'a, Message> = match tab {
        AssignTab::A => encoder_turn_body(&settings.encoder.turn, active),
        AssignTab::B => cc_slot_body(
            &settings.buttons.0[Buttons::EncoderPress as usize].press,
            EditField::EncoderPushCc,
            EditField::EncoderPushChannel,
            Message::SetEncoderPushType,
            active,
        ),
        AssignTab::C => cc_slot_body(
            &settings.buttons.0[Buttons::EncoderTouch as usize].press,
            EditField::EncoderTouchCc,
            EditField::EncoderTouchChannel,
            Message::SetEncoderTouchType,
            active,
        ),
    };
    column![header("Encoder", assignment), strip, group_box(body)]
        .spacing(10)
        .into()
}

fn encoder_turn_body<'a>(
    turn: &EncoderTurnAction,
    active: &dyn Fn(EditField) -> Option<String>,
) -> Element<'a, Message> {
    let ty = cc_type_of_encoder(turn);
    let type_row = labeled_pick_list(
        "Type",
        &CcType::ALL[..],
        Some(ty),
        Message::SetEncoderTurnType,
    );
    let mut col = column![type_row].spacing(8);
    if let EncoderTurnAction::Cc { channel, cc, mode } = turn {
        let channel = channel.map(|c| c.as_u8());
        col = col
            .push(channel_num_row(
                EditField::EncoderChannel,
                Some((channel.unwrap_or(0) + 1) as i8),
                active(EditField::EncoderChannel).as_deref(),
            ))
            .push(num_row(
                "Number",
                EditField::EncoderCc,
                Some(*cc as i8),
                active(EditField::EncoderCc).as_deref(),
            ))
            .push(labeled_pick_list(
                "Mode",
                &EncoderModeKind::ALL[..],
                Some(EncoderModeKind::of(mode)),
                Message::SetEncoderModeKind,
            ));
        match mode {
            settings::CcValueMode::Absolute { lo, hi, step, wrap } => {
                col = col
                    .push(range_row(
                        *lo,
                        active(EditField::EncoderLo).as_deref(),
                        *hi,
                        active(EditField::EncoderHi).as_deref(),
                    ))
                    .push(num_row(
                        "Step",
                        EditField::EncoderStep,
                        Some(*step),
                        active(EditField::EncoderStep).as_deref(),
                    ))
                    .push(
                        row![
                            text("Wrap").width(Length::Fixed(90.0)),
                            checkbox(*wrap).on_toggle(Message::SetEncoderWrap),
                        ]
                        .spacing(8)
                        .align_y(iced::alignment::Vertical::Center),
                    );
            }
            settings::CcValueMode::Relative { step }
            | settings::CcValueMode::RelativeOffset { step } => {
                col = col.push(num_row(
                    "Step",
                    EditField::EncoderStep,
                    Some(*step),
                    active(EditField::EncoderStep).as_deref(),
                ));
            }
        }
    }
    col.into()
}

/// A CC-or-Off body for a button-like slot (encoder push/touch). `on_type` sets its Type.
fn cc_slot_body<'a>(
    press: &ButtonPressAction,
    cc_field: EditField,
    ch_field: EditField,
    on_type: impl Fn(CcType) -> Message + 'a,
    active: &dyn Fn(EditField) -> Option<String>,
) -> Element<'a, Message> {
    let ty = cc_type_of_button(press);
    let type_row = labeled_pick_list("Type", &CcType::ALL[..], Some(ty), on_type);
    let mut col = column![type_row].spacing(8);
    if let ButtonPressAction::Cc { channel, cc } = press {
        let channel = channel.map(|c| c.as_u8());
        col = col
            .push(channel_num_row(
                ch_field,
                Some((channel.unwrap_or(0) + 1) as i8),
                active(ch_field).as_deref(),
            ))
            .push(num_row(
                "Number",
                cc_field,
                Some(*cc as i8),
                active(cc_field).as_deref(),
            ));
    }
    col.into()
}

/// Button Assign form over the selection (1+ buttons). Differing fields show
/// `…`; the Channel + Number rows appear only when every selected button is CC.
pub fn button_form<'a>(
    state: &State,
    name: &str,
    assignment: &str,
    active: &dyn Fn(EditField) -> Option<String>,
) -> Element<'a, Message> {
    let ty = state.buttons_cc_type();
    let selected = ty.value().map(|is_cc| {
        if is_cc {
            CcType::ControlChange
        } else {
            CcType::Off
        }
    });
    let type_row = labeled_pick_list("Type", &CcType::ALL[..], selected, Message::SetButtonType);
    let mut col = column![type_row].spacing(8);
    if ty == MultiValue::Same(true) {
        col = col
            .push(channel_num_row(
                EditField::ButtonChannel,
                state.buttons_channel().value().map(|v| v as i8),
                active(EditField::ButtonChannel).as_deref(),
            ))
            .push(num_row(
                "Number",
                EditField::ButtonCc,
                state.buttons_cc().value().map(|v| v as i8),
                active(EditField::ButtonCc).as_deref(),
            ));
    }
    column![header(name, assignment), group_box(col)]
        .spacing(10)
        .into()
}

/// Pad Assign form with Hit | Press sub-action tabs, over the selection (1+
/// pads). Differing fields show `…`; the Channel + Note rows appear only when
/// every selected pad shares the sending action type (all Note / all Poly).
pub fn pad_form<'a>(
    state: &State,
    name: &str,
    assignment: &str,
    tab: AssignTab,
    active: &dyn Fn(EditField) -> Option<String>,
) -> Element<'a, Message> {
    let body: Element<'a, Message> = match tab {
        AssignTab::A | AssignTab::C => {
            let ty = state.pads_hit_type();
            let selected = ty.value().map(|is_note| {
                if is_note {
                    PadHitType::Note
                } else {
                    PadHitType::Off
                }
            });
            let type_row = labeled_pick_list(
                "Type",
                &PadHitType::ALL[..],
                selected,
                Message::SetPadHitType,
            );
            let mut col = column![type_row].spacing(8);
            if ty == MultiValue::Same(true) {
                col = col
                    .push(channel_num_row(
                        EditField::PadHitChannel,
                        state.pads_hit_channel().value().map(|v| v as i8),
                        active(EditField::PadHitChannel).as_deref(),
                    ))
                    .push(num_row(
                        "Note",
                        EditField::PadHitNote,
                        state.pads_hit_note().value().map(|v| v as i8),
                        active(EditField::PadHitNote).as_deref(),
                    ));
            }
            col.into()
        }
        AssignTab::B => {
            let ty = state.pads_press_type();
            let selected = ty.value().map(|is_poly| {
                if is_poly {
                    PadPressType::PolyPressure
                } else {
                    PadPressType::Off
                }
            });
            let type_row = labeled_pick_list(
                "Type",
                &PadPressType::ALL[..],
                selected,
                Message::SetPadPressType,
            );
            let mut col = column![type_row].spacing(8);
            if ty == MultiValue::Same(true) {
                col = col
                    .push(channel_num_row(
                        EditField::PadPressChannel,
                        state.pads_press_channel().value().map(|v| v as i8),
                        active(EditField::PadPressChannel).as_deref(),
                    ))
                    .push(num_row(
                        "Note",
                        EditField::PadPressNote,
                        state.pads_press_note().value().map(|v| v as i8),
                        active(EditField::PadPressNote).as_deref(),
                    ));
            }
            col.into()
        }
    };

    column![
        header(name, assignment),
        tab_strip(&[("Hit", AssignTab::A), ("Press", AssignTab::B)], tab),
        group_box(body),
        group_box(pad_led_section(state)),
    ]
    .spacing(10)
    .into()
}

/// A labeled `pick_list` row: `label  [ value ▾ ]`. `selected` is the shared
/// value, or `None` (indeterminate → placeholder) across a multi-selection.
fn labeled_pick_list<'a, T>(
    label: &str,
    options: &'a [T],
    selected: Option<T>,
    on_select: impl Fn(T) -> Message + 'a,
) -> Element<'a, Message>
where
    T: ToString + PartialEq + Clone + 'a,
{
    row![
        text(label.to_string()).width(Length::Fixed(90.0)),
        pick_list(options, selected, on_select).placeholder("…"),
    ]
    .spacing(8)
    .align_y(iced::alignment::Vertical::Center)
    .into()
}

/// The pad LED section: LED On source dropdown, and the color fields for the
/// source being edited. The dropdown's selected source IS the source you edit;
/// the other source's stored colors persist in the schema but are not shown
/// until you switch the dropdown. Folds over the pad selection.
fn pad_led_section<'a>(state: &State) -> Element<'a, Message> {
    let source = state.pads_led_source().value();
    let source_row = labeled_pick_list(
        "LED On",
        &PadLedSource::ALL[..],
        source,
        Message::SetPadLedSource,
    );

    let mut col = column![text("LED").size(15), source_row].spacing(8);

    // The active source is the one being edited. Off / indeterminate selection
    // shows no color config (nothing to edit).
    let edit_tab = match source {
        Some(PadLedSource::MidiIn) => Some(LedTab::In),
        Some(PadLedSource::MidiOut) => Some(LedTab::Out),
        _ => None,
    };
    if let Some(tab) = edit_tab {
        let mode = state.pads_led_mode(tab).value();
        col = col.push(labeled_pick_list(
            "Color Mode",
            &PadLedMode::ALL[..],
            mode,
            move |m| Message::SetPadLedMode(tab, m),
        ));
        match mode {
            Some(PadLedMode::Single) => {
                // Off would make the lit state invisible; pick source `Off` to
                // disable the LED instead, so it is dropped from the hue list.
                col = col.push(labeled_pick_list(
                    "Color",
                    &PadColors::ALL[1..],
                    state.pads_led_single_color(tab).value(),
                    move |c| Message::SetPadLedColor(tab, PadLedColorSlot::Single, c),
                ));
            }
            Some(PadLedMode::Dual) => {
                col = col
                    .push(labeled_pick_list(
                        "Color On",
                        &PadColors::ALL[1..],
                        state.pads_led_dual_on(tab).value(),
                        move |c| Message::SetPadLedColor(tab, PadLedColorSlot::DualOn, c),
                    ))
                    .push(labeled_pick_list(
                        "Color Off",
                        &PadColors::ALL[..],
                        state.pads_led_dual_off(tab).value(),
                        move |c| Message::SetPadLedColor(tab, PadLedColorSlot::DualOff, c),
                    ));
            }
            _ => {}
        }
    }
    col.into()
}

/// The strip-wide LED section. `auto_off_active` is the live auto-off text
/// buffer if being typed.
fn led_section<'a>(led: SliderLedSettings, auto_off_active: Option<&str>) -> Element<'a, Message> {
    let auto_off_value = auto_off_active
        .map(str::to_string)
        .unwrap_or_else(|| led.auto_off_ms.to_string());
    column![
        text("LED").size(15),
        row![
            text("Mode"),
            pick_list(
                &SliderLedMode::ALL[..],
                Some(led.mode),
                Message::SetSliderLedMode
            )
        ]
        .spacing(8),
        row![
            text("Color"),
            pick_list(
                &PadColors::ALL[..],
                Some(led.color),
                Message::SetSliderLedColor
            )
        ]
        .spacing(8),
        row![
            text("Stylized"),
            checkbox(led.stylized).on_toggle(Message::SetSliderLedStylized),
        ]
        .spacing(8)
        .align_y(iced::alignment::Vertical::Center),
        row![
            text("Auto-off (ms)"),
            text_input("0", &auto_off_value)
                .on_input(|s| Message::NumericInput(EditField::SliderAutoOff, s))
                .on_submit(Message::NumericCommit(EditField::SliderAutoOff))
                .width(Length::Fixed(100.0)),
        ]
        .spacing(8),
    ]
    .spacing(8)
    .into()
}

fn touch_param_rows<'a>(
    channel: Option<u8>,
    number_label: &str,
    number: u8,
    on_value: u8,
    off_value: u8,
    active: &dyn Fn(EditField) -> Option<String>,
) -> Element<'a, Message> {
    column![
        channel_num_row(
            EditField::SliderTouchChannel,
            Some((channel.unwrap_or(0) + 1) as i8),
            active(EditField::SliderTouchChannel).as_deref()
        ),
        num_row(
            number_label,
            EditField::SliderTouchNumber,
            Some(number as i8),
            active(EditField::SliderTouchNumber).as_deref()
        ),
        num_row(
            "On value",
            EditField::SliderTouchOn,
            Some(on_value as i8),
            active(EditField::SliderTouchOn).as_deref()
        ),
        num_row(
            "Off value",
            EditField::SliderTouchOff,
            Some(off_value as i8),
            active(EditField::SliderTouchOff).as_deref()
        ),
    ]
    .spacing(8)
    .into()
}

/// Slider Assign form: Position | Touch tabs + a strip-wide LED section.
/// `auto_off_active` is the live auto-off text buffer if being typed.
#[allow(clippy::too_many_arguments)]
pub fn slider_form<'a>(
    position: &SliderPositionAction,
    touch: &SliderTouchAction,
    led: SliderLedSettings,
    assignment: &str,
    tab: AssignTab,
    auto_off_active: Option<&str>,
    active: &dyn Fn(EditField) -> Option<String>,
) -> Element<'a, Message> {
    let body: Element<'a, Message> = match tab {
        AssignTab::A | AssignTab::C => {
            let type_row = labeled_pick_list(
                "Type",
                &CcType::ALL[..],
                Some(cc_type_of_position(position)),
                Message::SetSliderPositionType,
            );
            let mut col = column![type_row].spacing(8);
            if let SliderPositionAction::Cc { channel, cc } = position {
                let channel = channel.map(|c| c.as_u8());
                col = col
                    .push(channel_num_row(
                        EditField::SliderChannel,
                        Some((channel.unwrap_or(0) + 1) as i8),
                        active(EditField::SliderChannel).as_deref(),
                    ))
                    .push(num_row(
                        "Number",
                        EditField::SliderCc,
                        Some(*cc as i8),
                        active(EditField::SliderCc).as_deref(),
                    ));
            }
            col.into()
        }
        AssignTab::B => {
            let type_row = labeled_pick_list(
                "Type",
                &SliderTouchKind::ALL[..],
                Some(SliderTouchKind::of(touch)),
                Message::SetSliderTouchKind,
            );
            let mut col = column![type_row].spacing(8);
            match touch {
                SliderTouchAction::Disabled => {}
                SliderTouchAction::Note {
                    channel,
                    note,
                    on_value,
                    off_value,
                } => {
                    col = col.push(touch_param_rows(
                        channel.map(|c| c.as_u8()),
                        "Note",
                        *note,
                        *on_value,
                        *off_value,
                        active,
                    ))
                }
                SliderTouchAction::Cc {
                    channel,
                    cc,
                    on_value,
                    off_value,
                } => {
                    col = col.push(touch_param_rows(
                        channel.map(|c| c.as_u8()),
                        "Number",
                        *cc,
                        *on_value,
                        *off_value,
                        active,
                    ))
                }
            }
            col.into()
        }
    };
    column![
        header("Touchstrip", assignment),
        tab_strip(&[("Position", AssignTab::A), ("Touch", AssignTab::B)], tab,),
        group_box(body),
        group_box(led_section(led, auto_off_active)),
    ]
    .spacing(12)
    .into()
}

/// The assignment label shown in a multi-selection header: the common label, or
/// `…` when the per-control labels differ.
fn indeterminate_label(labels: impl IntoIterator<Item = String>) -> String {
    match fold(labels) {
        MultiValue::Same(label) => label,
        MultiValue::Differ => "…".to_string(),
    }
}

/// Typed assignment form for the current selection, batching over same-type controls.
pub fn assignment_body(state: &State) -> Element<'_, Message> {
    let Some(s) = state.settings.as_ref() else {
        return column![text("Assignment").size(18), text("Connecting…")]
            .spacing(8)
            .into();
    };

    let edit_field = state.edit_field;
    let edit_text = state.edit_text.clone();
    let active = move |f: EditField| -> Option<String> {
        (edit_field == Some(f)).then(|| edit_text.clone())
    };
    let auto_off_active =
        (state.edit_field == Some(EditField::SliderAutoOff)).then(|| state.edit_text.clone());

    let pads = state.selected_pads();
    let buttons = state.selected_buttons();

    if !pads.is_empty() {
        let name = if pads.len() == 1 {
            crate::device::hotspots::control_name(ControlRef::Pad(pads[0]))
        } else {
            format!("{} pads", pads.len())
        };
        let label = indeterminate_label(pads.iter().map(|&p| {
            crate::device::labels::subaction_label(s, ControlRef::Pad(p), state.assign_tab)
        }));
        return pad_form(state, &name, &label, state.assign_tab, &active);
    }
    if !buttons.is_empty() {
        let name = if buttons.len() == 1 {
            crate::device::hotspots::control_name(ControlRef::Button(buttons[0]))
        } else {
            format!("{} buttons", buttons.len())
        };
        let label = indeterminate_label(
            buttons
                .iter()
                .map(|&b| crate::device::labels::control_label(s, ControlRef::Button(b))),
        );
        return button_form(state, &name, &label, &active);
    }
    match state.selection.first() {
        Some(ControlRef::Encoder) => {
            let label =
                crate::device::labels::subaction_label(s, ControlRef::Encoder, state.assign_tab);
            encoder_form(s, &label, state.assign_tab, &active)
        }
        Some(ControlRef::Slider) => {
            let label =
                crate::device::labels::subaction_label(s, ControlRef::Slider, state.assign_tab);
            slider_form(
                &s.slider.position,
                &s.slider.touch,
                s.slider.led,
                &label,
                state.assign_tab,
                auto_off_active.as_deref(),
                &active,
            )
        }
        _ => column![
            text("Assignment").size(18),
            text("Select a control on the device.")
        ]
        .spacing(8)
        .into(),
    }
}

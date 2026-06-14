use std::ops::RangeInclusive;

use iced::widget::{mouse_area, text_input};
use iced::{Element, Length};

use crate::inspector::assign::numeric::EditField;
use crate::message::Message;

/// Parse `input` as a signed integer clamped into `range`. Returns `None` for
/// empty or non-numeric input (caller keeps the prior value). A lone `-` while
/// typing a negative value parses as non-numeric, so the prior value is kept.
pub fn parse_clamped(input: &str, range: RangeInclusive<i8>) -> Option<i8> {
    let n: i32 = input.trim().parse().ok()?;
    Some(n.clamp(*range.start() as i32, *range.end() as i32) as i8)
}

/// Step `value` by `dir` (+1 / -1 / 0), saturating and clamped to `range`.
pub fn step_value(value: i8, dir: i8, range: RangeInclusive<i8>) -> i8 {
    let stepped = match dir {
        d if d > 0 => value.saturating_add(1),
        d if d < 0 => value.saturating_sub(1),
        _ => value,
    };
    stepped.clamp(*range.start(), *range.end())
}

/// Direction of a scroll delta: +1 up, -1 down, 0 otherwise.
pub fn scroll_sign(delta: iced::mouse::ScrollDelta) -> i8 {
    let y = match delta {
        iced::mouse::ScrollDelta::Lines { y, .. } => y,
        iced::mouse::ScrollDelta::Pixels { y, .. } => y,
    };
    if y > 0.0 {
        1
    } else if y < 0.0 {
        -1
    } else {
        0
    }
}

/// A numeric text box for `field`. Shows `active` (live edit buffer) when this
/// field is being typed; else the value, or an indeterminate `…` placeholder
/// when `value` is `None` (a multi-selection whose values differ). Type to set
/// all selected controls; scroll to step ±1 (no-op while indeterminate).
pub fn numeric_field<'a>(
    field: EditField,
    value: Option<i8>,
    active: Option<&str>,
) -> Element<'a, Message> {
    let shown = active
        .map(str::to_string)
        .or_else(|| value.map(|v| v.to_string()))
        .unwrap_or_default();
    let placeholder = if active.is_none() && value.is_none() {
        "…"
    } else {
        ""
    };
    let input = text_input(placeholder, &shown)
        .on_input(move |s| Message::NumericInput(field, s))
        .on_submit(Message::NumericCommit(field))
        .width(Length::Fixed(64.0));
    mouse_area(input)
        .on_scroll(move |d| Message::NumericStep(field, scroll_sign(d)))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clamped_handles_valid_clamped_and_garbage() {
        assert_eq!(parse_clamped("64", 0..=127), Some(64));
        assert_eq!(parse_clamped("200", 0..=127), Some(127));
        assert_eq!(parse_clamped(" 5 ", 1..=16), Some(5));
        assert_eq!(parse_clamped("0", 1..=16), Some(1)); // clamps up into channel range
        assert_eq!(parse_clamped("-40", i8::MIN..=i8::MAX), Some(-40)); // signed step
        assert_eq!(parse_clamped("-50", -32..=31), Some(-32)); // clamps into relative range
        assert_eq!(parse_clamped("-", i8::MIN..=i8::MAX), None); // lone minus: keep prior
        assert_eq!(parse_clamped("", 0..=127), None);
        assert_eq!(parse_clamped("x", 0..=127), None);
    }

    #[test]
    fn step_value_saturates_at_bounds() {
        assert_eq!(step_value(10, 1, 0..=127), 11);
        assert_eq!(step_value(10, -1, 0..=127), 9);
        assert_eq!(step_value(127, 1, 0..=127), 127);
        assert_eq!(step_value(1, -1, 1..=16), 1);
        assert_eq!(step_value(5, 0, 0..=127), 5);
        assert_eq!(step_value(-5, -1, i8::MIN..=i8::MAX), -6); // signed step
    }
}

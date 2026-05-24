use maschine_library::controls::PadEventType;
use num::FromPrimitive;

use crate::events::ControlEvent;
use crate::velocity::{PadVelocityCurve, pad_velocity};

pub struct ControlState {
    buttons: [bool; 41],
    slider_value: u8,
    encoder_pos: Option<u8>,
    suppress_encoder_packet: bool,
    last_aftertouch: [Option<u8>; 16],
    slider_touched: bool,
}

impl ControlState {
    pub fn new() -> Self {
        Self {
            buttons: [false; 41],
            slider_value: 0,
            encoder_pos: None,
            suppress_encoder_packet: false,
            last_aftertouch: [None; 16],
            slider_touched: false,
        }
    }
}

impl Default for ControlState {
    fn default() -> Self {
        Self::new()
    }
}

fn scale_pressure_to_midi(val: u16) -> u8 {
    (val >> 5).min(127) as u8
}

pub fn decode_packet(state: &mut ControlState, buf: &[u8; 64]) -> Vec<ControlEvent> {
    decode_packet_with_curve(state, buf, PadVelocityCurve::Linear)
}

pub(crate) fn decode_packet_with_curve(
    state: &mut ControlState,
    buf: &[u8; 64],
    curve: PadVelocityCurve,
) -> Vec<ControlEvent> {
    let mut events = Vec::new();

    match buf[0] {
        0x01 => {
            let mut encoder_touch_just_pressed = false;

            for i in 0..6 {
                for j in 0..8 {
                    let idx = i * 8 + j;
                    if idx >= state.buttons.len() {
                        continue;
                    }

                    let pressed = (buf[i + 1] & (1 << j)) != 0;
                    if pressed != state.buttons[idx] {
                        state.buttons[idx] = pressed;
                        events.push(ControlEvent::ButtonChanged {
                            index: idx,
                            pressed,
                        });

                        if idx == 40 && pressed {
                            encoder_touch_just_pressed = true;
                        }
                    }
                }
            }

            let cur_pos = buf[7] & 0x0f;
            let suppress_encoder = encoder_touch_just_pressed || state.suppress_encoder_packet;

            if suppress_encoder {
                state.encoder_pos = Some(cur_pos);
            } else if let Some(prev_pos) = state.encoder_pos {
                let diff = cur_pos.wrapping_sub(prev_pos) & 0x0f;
                let delta = if diff < 8 {
                    diff as i8
                } else {
                    diff as i8 - 16
                };
                if delta != 0 {
                    let cc_value = (64i16 + delta as i16).clamp(0, 127) as u8;
                    events.push(ControlEvent::EncoderTurn { delta, cc_value });
                }
                state.encoder_pos = Some(cur_pos);
            } else {
                state.encoder_pos = Some(cur_pos);
            }

            if encoder_touch_just_pressed {
                state.suppress_encoder_packet = true;
            } else if state.suppress_encoder_packet {
                state.suppress_encoder_packet = false;
            }

            // Slider touch: the device reports buf[10] = 0 when the slider is
            // not touched and buf[10] = position (1..=200) while touched.
            // Confirmed from wireshark/slider_touch_event.pcapng: touch-on frame
            // has buf[10] = 0x5b (91), touch-off frame has buf[10] = 0x00.
            // There is no separate dedicated touch bit; the zero/non-zero state of
            // the cooked position byte serves as the touch indicator.
            let slider_raw = buf[10];
            let slider_touched = slider_raw != 0;
            if slider_touched != state.slider_touched {
                state.slider_touched = slider_touched;
                events.push(ControlEvent::SliderTouch {
                    pressed: slider_touched,
                });
            }

            if slider_touched && slider_raw != state.slider_value {
                state.slider_value = slider_raw;
                let cc_value = ((slider_raw as u16 - 1) * 127 / 200).min(127) as u8;
                events.push(ControlEvent::SliderMoved {
                    raw: slider_raw,
                    cc_value,
                });
            }
        }
        0x02 => {
            for i in (1..buf.len()).step_by(3) {
                let idx = buf[i] as usize;
                let evt = buf[i + 1] & 0xf0;
                let val = ((buf[i + 1] as u16 & 0x0f) << 8) + buf[i + 2] as u16;

                if i > 1 && idx == 0 && evt == 0 && val == 0 {
                    break;
                }

                let Some(pad_evt) = PadEventType::from_u8(evt) else {
                    continue;
                };

                if idx >= 16 {
                    continue;
                }

                let velocity = pad_velocity(val, curve);

                match pad_evt {
                    PadEventType::NoteOn | PadEventType::PressOn => {
                        events.push(ControlEvent::PadNoteOn {
                            index: idx,
                            velocity,
                        });
                    }
                    PadEventType::NoteOff | PadEventType::PressOff => {
                        state.last_aftertouch[idx] = None;
                        events.push(ControlEvent::PadNoteOff {
                            index: idx,
                            velocity,
                        });
                    }
                    PadEventType::Aftertouch => {
                        let pressure = scale_pressure_to_midi(val);
                        if state.last_aftertouch[idx] != Some(pressure) {
                            state.last_aftertouch[idx] = Some(pressure);
                            events.push(ControlEvent::PadAftertouch {
                                index: idx,
                                pressure,
                            });
                        }
                    }
                }
            }
        }
        _ => {}
    }

    events
}

#[cfg(test)]
mod tests {
    use super::{ControlState, decode_packet, decode_packet_with_curve};
    use crate::events::ControlEvent;
    use crate::velocity::PadVelocityCurve;

    #[test]
    fn decodes_button_press_to_event() {
        let mut state = ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x01;
        buf[1] = 0b0000_0001;

        let events = decode_packet(&mut state, &buf);

        assert_eq!(
            events,
            vec![ControlEvent::ButtonChanged {
                index: 0,
                pressed: true
            }]
        );
    }

    #[test]
    fn decodes_slider_change_to_event() {
        let mut state = ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x01;
        buf[10] = 101;

        let events = decode_packet(&mut state, &buf);

        // First contact emits both SliderTouch (pressed) and SliderMoved.
        assert_eq!(
            events,
            vec![
                ControlEvent::SliderTouch { pressed: true },
                ControlEvent::SliderMoved {
                    raw: 101,
                    cc_value: 63
                },
            ]
        );
    }

    #[test]
    fn repeated_same_button_packet_does_not_emit_duplicate_event() {
        let mut state = ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x01;
        buf[1] = 0b0000_0001;

        let first_events = decode_packet(&mut state, &buf);
        let second_events = decode_packet(&mut state, &buf);

        assert_eq!(
            first_events,
            vec![ControlEvent::ButtonChanged {
                index: 0,
                pressed: true
            }]
        );
        assert!(second_events.is_empty());
    }

    #[test]
    fn encoder_touch_packet_with_changed_position_does_not_emit_encoder_turn() {
        let mut state = ControlState::new();
        let mut initial = [0u8; 64];
        initial[0] = 0x01;
        initial[7] = 0x04;

        let mut touched = [0u8; 64];
        touched[0] = 0x01;
        touched[6] = 0b0000_0001;
        touched[7] = 0x05;

        let initial_events = decode_packet(&mut state, &initial);
        let touched_events = decode_packet(&mut state, &touched);

        assert!(initial_events.is_empty());
        assert_eq!(
            touched_events,
            vec![ControlEvent::ButtonChanged {
                index: 40,
                pressed: true
            }]
        );
    }

    #[test]
    fn encoder_delta_resumes_after_touch_suppression_packet() {
        let mut state = ControlState::new();
        let mut initial = [0u8; 64];
        initial[0] = 0x01;
        initial[7] = 0x04;

        let mut touched = [0u8; 64];
        touched[0] = 0x01;
        touched[6] = 0b0000_0001;
        touched[7] = 0x05;

        let mut suppressed_next = [0u8; 64];
        suppressed_next[0] = 0x01;
        suppressed_next[6] = 0b0000_0001;
        suppressed_next[7] = 0x06;

        let mut normal_turn = [0u8; 64];
        normal_turn[0] = 0x01;
        normal_turn[6] = 0b0000_0001;
        normal_turn[7] = 0x07;

        assert!(decode_packet(&mut state, &initial).is_empty());
        assert_eq!(
            decode_packet(&mut state, &touched),
            vec![ControlEvent::ButtonChanged {
                index: 40,
                pressed: true
            }]
        );
        assert!(decode_packet(&mut state, &suppressed_next).is_empty());
        assert_eq!(
            decode_packet(&mut state, &normal_turn),
            vec![ControlEvent::EncoderTurn {
                delta: 1,
                cc_value: 65
            }]
        );
    }

    #[test]
    fn decodes_pad_note_on_and_stops_at_packet_terminator() {
        let mut state = ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x02;
        buf[1] = 2;
        buf[2] = 0x10;
        buf[3] = 64;
        buf[4] = 0;
        buf[5] = 0;
        buf[6] = 0;
        buf[7] = 9;
        buf[8] = 0x10;
        buf[9] = 96;

        let events = decode_packet(&mut state, &buf);

        assert_eq!(
            events,
            vec![ControlEvent::PadNoteOn {
                index: 2,
                velocity: 2
            }]
        );
    }

    #[test]
    fn decodes_pad_note_on_with_hard_velocity_curve() {
        let mut state = ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x02;
        buf[1] = 2;
        buf[2] = 0x18;
        buf[3] = 0x00;

        let events = decode_packet_with_curve(&mut state, &buf, PadVelocityCurve::Hard3);

        assert_eq!(
            events,
            vec![ControlEvent::PadNoteOn {
                index: 2,
                velocity: 26
            }]
        );
    }

    #[test]
    fn decodes_pad_note_off_event() {
        let mut state = ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x02;
        buf[1] = 3;
        buf[2] = 0x30;
        buf[3] = 96;

        let events = decode_packet(&mut state, &buf);

        assert_eq!(
            events,
            vec![ControlEvent::PadNoteOff {
                index: 3,
                velocity: 3
            }]
        );
    }

    #[test]
    fn decodes_encoder_wraparound_as_forward_step() {
        let mut state = ControlState::new();
        let mut initial = [0u8; 64];
        initial[0] = 0x01;
        initial[7] = 0x0f;

        let mut wrapped = [0u8; 64];
        wrapped[0] = 0x01;
        wrapped[7] = 0x00;

        assert!(decode_packet(&mut state, &initial).is_empty());
        assert_eq!(
            decode_packet(&mut state, &wrapped),
            vec![ControlEvent::EncoderTurn {
                delta: 1,
                cc_value: 65
            }]
        );
    }

    #[test]
    fn drops_pad_events_with_out_of_range_index() {
        let mut state = ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x02;
        buf[1] = 16;
        buf[2] = 0x10;
        buf[3] = 64;

        let events = decode_packet(&mut state, &buf);

        assert!(events.is_empty());
    }

    #[test]
    fn decodes_aftertouch_to_event_with_scaled_pressure() {
        let mut state = ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x02;
        buf[1] = 4;
        buf[2] = 0x40; // PadEventType::Aftertouch
        buf[3] = 0xFF; // val low byte; high nibble is in buf[2] & 0x0f (0)
        // 12-bit val = 0x0FF = 255 → (255 >> 5) = 7

        let events = decode_packet(&mut state, &buf);
        assert_eq!(
            events,
            vec![ControlEvent::PadAftertouch {
                index: 4,
                pressure: 7
            }]
        );
    }

    #[test]
    fn aftertouch_min_endpoint_pressure_is_zero() {
        let mut state = ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x02;
        buf[1] = 4;
        buf[2] = 0x40;
        buf[3] = 0x00;
        // val = 0 → pressure = 0

        let events = decode_packet(&mut state, &buf);
        assert_eq!(
            events,
            vec![ControlEvent::PadAftertouch {
                index: 4,
                pressure: 0
            }]
        );
    }

    #[test]
    fn aftertouch_max_endpoint_pressure_is_127() {
        let mut state = ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x02;
        buf[1] = 4;
        buf[2] = 0x4F; // high nibble of 12-bit val = 0xF
        buf[3] = 0xFF; // low byte of 12-bit val
        // 12-bit val = 0xFFF = 4095 → (4095 >> 5) = 127

        let events = decode_packet(&mut state, &buf);
        assert_eq!(
            events,
            vec![ControlEvent::PadAftertouch {
                index: 4,
                pressure: 127
            }]
        );
    }

    #[test]
    fn aftertouch_duplicate_value_does_not_re_emit() {
        let mut state = ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x02;
        buf[1] = 4;
        buf[2] = 0x40;
        buf[3] = 0xFF;

        let _first = decode_packet(&mut state, &buf);
        let second = decode_packet(&mut state, &buf);
        assert!(
            second.is_empty(),
            "duplicate pressure should not re-emit, got {second:?}"
        );
    }

    #[test]
    fn decodes_slider_touch_press_then_release() {
        let mut state = ControlState::new();

        // Touch-on: buf[10] is any non-zero value (slider position while touched).
        let mut press = [0u8; 64];
        press[0] = 0x01;
        press[10] = 0x5b; // position 91, confirmed from wireshark/slider_touch_event.pcapng

        // Touch-off: buf[10] = 0 (device sends 0 when slider is released).
        let mut release = [0u8; 64];
        release[0] = 0x01;
        release[10] = 0;

        let press_events = decode_packet(&mut state, &press);
        let release_events = decode_packet(&mut state, &release);

        // (0x5b - 1) * 127 / 200 = 90 * 127 / 200 = 57
        assert_eq!(
            press_events,
            vec![
                ControlEvent::SliderTouch { pressed: true },
                ControlEvent::SliderMoved {
                    raw: 0x5b,
                    cc_value: 57
                },
            ]
        );
        assert_eq!(
            release_events,
            vec![ControlEvent::SliderTouch { pressed: false }]
        );
    }

    #[test]
    fn slider_touch_stable_state_does_not_re_emit() {
        let mut state = ControlState::new();

        let mut press = [0u8; 64];
        press[0] = 0x01;
        press[10] = 0x5b;

        let first = decode_packet(&mut state, &press);
        let second = decode_packet(&mut state, &press);

        assert!(
            first.contains(&ControlEvent::SliderTouch { pressed: true }),
            "expected SliderTouch on first packet, got {first:?}"
        );
        assert!(
            !second.contains(&ControlEvent::SliderTouch { pressed: true }),
            "expected no SliderTouch on stable touch, got {second:?}"
        );
    }

    #[test]
    fn aftertouch_state_resets_after_note_off_in_same_packet() {
        let mut state = ControlState::new();

        // First: aftertouch on pad 4 with pressure 7
        let mut at_buf = [0u8; 64];
        at_buf[0] = 0x02;
        at_buf[1] = 4;
        at_buf[2] = 0x40;
        at_buf[3] = 0xFF;
        let _ = decode_packet(&mut state, &at_buf);

        // Then a note-off on pad 4
        let mut off_buf = [0u8; 64];
        off_buf[0] = 0x02;
        off_buf[1] = 4;
        off_buf[2] = 0x30; // NoteOff
        off_buf[3] = 0x00;
        let _ = decode_packet(&mut state, &off_buf);

        // Same aftertouch should now emit again
        let again = decode_packet(&mut state, &at_buf);
        assert_eq!(
            again,
            vec![ControlEvent::PadAftertouch {
                index: 4,
                pressure: 7
            }]
        );
    }
}

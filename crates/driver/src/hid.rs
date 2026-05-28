use maschine_library::controls::PadEventType;
use num::FromPrimitive;

use crate::events::ControlEvent;

pub struct ControlState {
    buttons: [bool; 41],
    slider_value: u8,
    encoder_pos: Option<u8>,
    suppress_encoder_packet: bool,
}

impl ControlState {
    pub fn new() -> Self {
        Self {
            buttons: [false; 41],
            slider_value: 0,
            encoder_pos: None,
            suppress_encoder_packet: false,
        }
    }
}

impl Default for ControlState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn decode_packet(state: &mut ControlState, buf: &[u8; 64]) -> Vec<ControlEvent> {
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

            let slider_raw = buf[10];
            if slider_raw != 0 && slider_raw != state.slider_value {
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

                let mut velocity = (val >> 5) as u8;
                if val > 0 && velocity == 0 {
                    velocity = 1;
                }

                match pad_evt {
                    PadEventType::NoteOn | PadEventType::PressOn => {
                        events.push(ControlEvent::PadNoteOn {
                            index: idx,
                            velocity,
                        });
                    }
                    PadEventType::NoteOff | PadEventType::PressOff => {
                        events.push(ControlEvent::PadNoteOff {
                            index: idx,
                            velocity,
                        });
                    }
                    PadEventType::Aftertouch => {}
                }
            }
        }
        _ => {}
    }

    events
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_button_press_to_event() {
        let mut state = crate::hid::ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x01;
        buf[1] = 0b0000_0001;

        let events = crate::hid::decode_packet(&mut state, &buf);

        assert_eq!(
            events,
            vec![crate::events::ControlEvent::ButtonChanged {
                index: 0,
                pressed: true
            }]
        );
    }

    #[test]
    fn decodes_slider_change_to_event() {
        let mut state = crate::hid::ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x01;
        buf[10] = 101;

        let events = crate::hid::decode_packet(&mut state, &buf);

        assert_eq!(
            events,
            vec![crate::events::ControlEvent::SliderMoved {
                raw: 101,
                cc_value: 63
            }]
        );
    }

    #[test]
    fn repeated_same_button_packet_does_not_emit_duplicate_event() {
        let mut state = crate::hid::ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x01;
        buf[1] = 0b0000_0001;

        let first_events = crate::hid::decode_packet(&mut state, &buf);
        let second_events = crate::hid::decode_packet(&mut state, &buf);

        assert_eq!(
            first_events,
            vec![crate::events::ControlEvent::ButtonChanged {
                index: 0,
                pressed: true
            }]
        );
        assert!(second_events.is_empty());
    }

    #[test]
    fn encoder_touch_packet_with_changed_position_does_not_emit_encoder_turn() {
        let mut state = crate::hid::ControlState::new();
        let mut initial = [0u8; 64];
        initial[0] = 0x01;
        initial[7] = 0x04;

        let mut touched = [0u8; 64];
        touched[0] = 0x01;
        touched[6] = 0b0000_0001;
        touched[7] = 0x05;

        let initial_events = crate::hid::decode_packet(&mut state, &initial);
        let touched_events = crate::hid::decode_packet(&mut state, &touched);

        assert!(initial_events.is_empty());
        assert_eq!(
            touched_events,
            vec![crate::events::ControlEvent::ButtonChanged {
                index: 40,
                pressed: true
            }]
        );
    }

    #[test]
    fn encoder_delta_resumes_after_touch_suppression_packet() {
        let mut state = crate::hid::ControlState::new();
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

        assert!(crate::hid::decode_packet(&mut state, &initial).is_empty());
        assert_eq!(
            crate::hid::decode_packet(&mut state, &touched),
            vec![crate::events::ControlEvent::ButtonChanged {
                index: 40,
                pressed: true
            }]
        );
        assert!(crate::hid::decode_packet(&mut state, &suppressed_next).is_empty());
        assert_eq!(
            crate::hid::decode_packet(&mut state, &normal_turn),
            vec![crate::events::ControlEvent::EncoderTurn {
                delta: 1,
                cc_value: 65
            }]
        );
    }

    #[test]
    fn decodes_pad_note_on_and_stops_at_packet_terminator() {
        let mut state = crate::hid::ControlState::new();
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

        let events = crate::hid::decode_packet(&mut state, &buf);

        assert_eq!(
            events,
            vec![crate::events::ControlEvent::PadNoteOn {
                index: 2,
                velocity: 2
            }]
        );
    }

    #[test]
    fn decodes_pad_note_off_event() {
        let mut state = crate::hid::ControlState::new();
        let mut buf = [0u8; 64];
        buf[0] = 0x02;
        buf[1] = 3;
        buf[2] = 0x30;
        buf[3] = 96;

        let events = crate::hid::decode_packet(&mut state, &buf);

        assert_eq!(
            events,
            vec![crate::events::ControlEvent::PadNoteOff {
                index: 3,
                velocity: 3
            }]
        );
    }

    #[test]
    fn decodes_encoder_wraparound_as_forward_step() {
        let mut state = crate::hid::ControlState::new();
        let mut initial = [0u8; 64];
        initial[0] = 0x01;
        initial[7] = 0x0f;

        let mut wrapped = [0u8; 64];
        wrapped[0] = 0x01;
        wrapped[7] = 0x00;

        assert!(crate::hid::decode_packet(&mut state, &initial).is_empty());
        assert_eq!(
            crate::hid::decode_packet(&mut state, &wrapped),
            vec![crate::events::ControlEvent::EncoderTurn {
                delta: 1,
                cc_value: 65
            }]
        );
    }
}

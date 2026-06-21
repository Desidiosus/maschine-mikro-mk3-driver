use std::sync::Arc;

use maschine_library::lights::{BUTTON_BACKLIGHT_LEVEL, Brightness};
use midir::os::unix::{VirtualInput, VirtualOutput};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use protocol::{DriverToGui, MidiDir};

use crate::error::{DriverError, DriverResult};
use crate::events::ControlEvent;
use crate::feedback::midi::apply_incoming_midi_message;
use crate::ipc::{EventSubscriber, emit_event};
use crate::outputs::DeviceOutputs;
use crate::settings::actions::{
    ButtonPressAction, CcValueMode, EncoderTurnAction, PadHitAction, PadPressureAction,
    SliderPositionAction, SliderTouchAction,
};
use crate::settings::{MidiChannel, Settings};
use crate::shared_settings::{SharedSettings, new_shared};
use crate::soft_off::SoftOffSync;
use crate::virmidi_bridge::try_autoconnect_virmidi;

/// Downstream MIDI send step; lets tests substitute a capturing fake.
pub trait MidiSink {
    fn send(&mut self, bytes: &[u8]) -> DriverResult<()>;
}

impl MidiSink for MidiOutputConnection {
    fn send(&mut self, bytes: &[u8]) -> DriverResult<()> {
        MidiOutputConnection::send(self, bytes)
            .map_err(|err| DriverError::Midi(format!("failed to send MIDI message: {err}")))
    }
}

pub struct MidiBackend<S: MidiSink = MidiOutputConnection> {
    settings: SharedSettings,
    sink: S,
    _input: Option<MidiInputConnection<DeviceOutputs>>,
}

impl MidiBackend {
    pub fn new(
        settings: &SharedSettings,
        outputs: &DeviceOutputs,
        soft_off: SoftOffSync,
        runtime_state: crate::runtime_state::RuntimeState,
        subscriber: EventSubscriber,
    ) -> DriverResult<Self> {
        let snapshot = settings.load();
        let sink = MidiOutput::new(&snapshot.global.client_name)
            .map_err(|err| DriverError::Midi(format!("couldn't open MIDI output: {err}")))?
            .create_virtual(&snapshot.global.port_name)
            .map_err(|err| {
                DriverError::Midi(format!("couldn't create virtual output port: {err}"))
            })?;

        let input = create_midi_input(
            settings,
            outputs.clone(),
            soft_off,
            runtime_state,
            subscriber,
        )?;

        if snapshot.bridge.midi_bridge_virmidi && snapshot.bridge.autoconnect_virmidi {
            try_autoconnect_virmidi(&snapshot)?;
        }

        Ok(Self {
            settings: Arc::clone(settings),
            sink,
            _input: Some(input),
        })
    }
}

impl<S: MidiSink> MidiBackend<S> {
    /// Construct a backend around an arbitrary sink, without opening a
    /// MIDI input port. Intended for tests.
    pub fn with_sink(settings: Settings, sink: S) -> Self {
        Self {
            settings: new_shared(settings),
            sink,
            _input: None,
        }
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn handle_event(
        &mut self,
        event: &ControlEvent,
        rt: &crate::runtime_state::RuntimeState,
    ) -> DriverResult<bool> {
        let snapshot = self.settings.load();
        match event_to_midi_bytes(event, &snapshot, rt) {
            Some(bytes) => {
                self.sink.send(&bytes)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

fn create_midi_input(
    settings: &SharedSettings,
    outputs: DeviceOutputs,
    soft_off: SoftOffSync,
    runtime_state: crate::runtime_state::RuntimeState,
    subscriber: EventSubscriber,
) -> DriverResult<MidiInputConnection<DeviceOutputs>> {
    let settings_handle = Arc::clone(settings);
    let runtime_state_clone = runtime_state.clone();
    let snapshot = settings.load();
    let client_name = format!("{} In", snapshot.global.client_name);
    let port_name_in = snapshot.global.port_name_in.clone();
    drop(snapshot);

    MidiInput::new(&client_name)
        .map_err(|err| DriverError::Midi(format!("couldn't open MIDI input: {err}")))?
        .create_virtual(
            &port_name_in,
            move |_timestamp, message, outputs| {
                let _guard = soft_off.lock();
                if soft_off.is_active() {
                    return;
                }
                apply_incoming_midi_message(
                    message,
                    outputs,
                    &settings_handle.load(),
                    &runtime_state_clone,
                );
                emit_event(&subscriber, DriverToGui::MidiActivity { dir: MidiDir::In });
            },
            outputs,
        )
        .map_err(|err| DriverError::Midi(format!("couldn't create virtual input port: {err}")))
}

fn resolve_channel(per_action: Option<MidiChannel>) -> u8 {
    per_action.map(|c| c.as_u8()).unwrap_or(0)
}

/// NI relative (sign-magnitude) turn encoding: forward turns emit
/// `1..=REL_MAX_MAGNITUDE`, backward turns emit `REL_SIGN_PIVOT - magnitude`
/// (i.e. 65..=127), and `0` means no movement.
const REL_MAX_MAGNITUDE: u16 = 63;
const REL_SIGN_PIVOT: u8 = 128;
/// Relative-offset encoding centers the emitted CC value at the 7-bit midpoint.
const REL_OFFSET_CENTER: i16 = 64;

fn step_absolute(cur: u8, delta: i8, lo: u8, hi: u8, step: i8, wrap: bool) -> u8 {
    let span = (hi as i32 - lo as i32) + 1;
    let move_ = delta as i32 * step as i32;
    let off = cur as i32 - lo as i32 + move_;
    let new_off = if wrap {
        off.rem_euclid(span)
    } else {
        off.clamp(0, span - 1)
    };
    (lo as i32 + new_off) as u8
}

/// Translate an encoder `delta` (turn ticks, signed) into the CC value byte to
/// emit, per the configured `CcValueMode`. In `Absolute` mode the next value is
/// also written back into `rt` so subsequent turns continue from the current
/// position.
fn encode_encoder_value(
    mode: &CcValueMode,
    delta: i8,
    rt: &crate::runtime_state::RuntimeState,
) -> u8 {
    match mode {
        CcValueMode::Absolute { lo, hi, step, wrap } => {
            let next = step_absolute(rt.encoder_value(), delta, *lo, *hi, *step, *wrap);
            rt.set_encoder_value(next);
            next
        }
        CcValueMode::Relative { step } => {
            // A negative `step` reverses direction; the emitted sign is the sign
            // of `delta * step`. NI relative encoding: 1..=63 forward, 65..=127
            // backward (sign-magnitude around 0/128).
            let signed = delta as i16 * *step as i16;
            let mag = signed.unsigned_abs().min(REL_MAX_MAGNITUDE) as u8;
            if signed >= 0 {
                mag
            } else {
                REL_SIGN_PIVOT.wrapping_sub(mag)
            }
        }
        CcValueMode::RelativeOffset { step } => {
            let off = delta as i16 * *step as i16;
            (REL_OFFSET_CENTER + off).clamp(
                i16::from(CcValueMode::CC_VALUE_MIN),
                i16::from(CcValueMode::CC_VALUE_MAX),
            ) as u8
        }
    }
}

pub fn event_to_midi_bytes(
    event: &ControlEvent,
    settings: &Settings,
    rt: &crate::runtime_state::RuntimeState,
) -> Option<[u8; 3]> {
    match event {
        ControlEvent::ButtonChanged { index, pressed } => {
            let btn = settings.buttons.0.get(*index)?;
            match &btn.press {
                ButtonPressAction::Cc { channel, cc } => Some([
                    0xB0 | resolve_channel(*channel),
                    *cc,
                    if *pressed { 127 } else { 0 },
                ]),
                ButtonPressAction::Off => None,
            }
        }
        ControlEvent::EncoderTurn { delta, .. } => match &settings.encoder.turn {
            EncoderTurnAction::Cc { channel, cc, mode } => {
                let value = encode_encoder_value(mode, *delta, rt);
                Some([0xB0 | resolve_channel(*channel), *cc, value])
            }
            EncoderTurnAction::Off => None,
        },
        ControlEvent::SliderMoved { cc_value, .. } => match &settings.slider.position {
            SliderPositionAction::Cc { channel, cc } => {
                Some([0xB0 | resolve_channel(*channel), *cc, *cc_value])
            }
            SliderPositionAction::Off => None,
        },
        ControlEvent::SliderTouch { pressed } => match &settings.slider.touch {
            SliderTouchAction::Disabled => None,
            SliderTouchAction::Note {
                channel,
                note,
                on_value,
                off_value,
            } => Some(if *pressed {
                [0x90 | resolve_channel(*channel), *note, *on_value]
            } else {
                [0x80 | resolve_channel(*channel), *note, *off_value]
            }),
            SliderTouchAction::Cc {
                channel,
                cc,
                on_value,
                off_value,
            } => Some([
                0xB0 | resolve_channel(*channel),
                *cc,
                if *pressed { *on_value } else { *off_value },
            ]),
        },
        ControlEvent::PadNoteOn { index, velocity } => {
            let pad = settings.pads.0.get(*index)?;
            match &pad.hit {
                PadHitAction::Note { channel, note } => {
                    Some([0x90 | resolve_channel(*channel), *note, *velocity])
                }
                PadHitAction::Off => None,
            }
        }
        ControlEvent::PadNoteOff { index, velocity } => {
            let pad = settings.pads.0.get(*index)?;
            match &pad.hit {
                PadHitAction::Note { channel, note } => {
                    Some([0x80 | resolve_channel(*channel), *note, *velocity])
                }
                PadHitAction::Off => None,
            }
        }
        ControlEvent::PadAftertouch { index, pressure } => {
            let pad = settings.pads.0.get(*index)?;
            match &pad.pressure {
                PadPressureAction::Disabled => None,
                PadPressureAction::Poly { channel, note } => {
                    let resolved_note = match note {
                        Some(n) => *n,
                        None => match &pad.hit {
                            PadHitAction::Note { note, .. } => *note,
                            PadHitAction::Off => return None,
                        },
                    };
                    Some([0xA0 | resolve_channel(*channel), resolved_note, *pressure])
                }
            }
        }
    }
}

/// Locate the index of a control whose `(channel, key)` pair matches `target`.
/// `extract` pulls the per-action channel override and the routing key (note,
/// CC, …) from each control's action slot. A `None` channel resolves to
/// channel 0 (displayed channel 1).
fn find_index_for<I, T, F>(items: I, target: (u8, u8), extract: F) -> Option<usize>
where
    I: IntoIterator<Item = T>,
    F: Fn(T) -> Option<(Option<MidiChannel>, u8)>,
{
    let (channel, key) = target;
    items.into_iter().enumerate().find_map(|(idx, item)| {
        let (chan, item_key) = extract(item)?;
        let resolved = chan.map(|c| c.as_u8()).unwrap_or(0);
        (resolved == channel && item_key == key).then_some(idx)
    })
}

pub fn pad_index_for_message(settings: &Settings, channel: u8, note: u8) -> Option<usize> {
    find_index_for(settings.pads.iter(), (channel, note), |pad| {
        match &pad.hit {
            PadHitAction::Note { channel, note } => Some((*channel, *note)),
            PadHitAction::Off => None,
        }
    })
}

pub fn button_index_for_message(settings: &Settings, channel: u8, cc: u8) -> Option<usize> {
    find_index_for(settings.buttons.0.iter(), (channel, cc), |btn| {
        match &btn.press {
            ButtonPressAction::Cc { channel, cc } => Some((*channel, *cc)),
            ButtonPressAction::Off => None,
        }
    })
}

pub fn button_brightness_from_value(value: u8, backlight_on: bool) -> Brightness {
    if value > 0 {
        match value {
            1..=42 => Brightness::Dim,
            43..=84 => Brightness::Normal,
            _ => Brightness::Bright,
        }
    } else if backlight_on {
        BUTTON_BACKLIGHT_LEVEL
    } else {
        Brightness::Off
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::ControlEvent;
    use crate::settings::actions::{
        ButtonPressAction, CcValueMode, EncoderTurnAction, PadHitAction, PadPressureAction,
        SliderTouchAction,
    };
    use crate::settings::{MidiChannel, Settings};

    fn rt() -> crate::runtime_state::RuntimeState {
        crate::runtime_state::RuntimeState::default()
    }

    #[derive(Default)]
    struct CapturingSink {
        sent: Vec<Vec<u8>>,
    }

    impl MidiSink for CapturingSink {
        fn send(&mut self, bytes: &[u8]) -> DriverResult<()> {
            self.sent.push(bytes.to_vec());
            Ok(())
        }
    }

    fn settings_with_pad_pressure_enabled(idx: usize, channel: u8, note: Option<u8>) -> Settings {
        let mut s = Settings::default();
        s.pads[idx].pressure = PadPressureAction::Poly {
            channel: MidiChannel::try_from(channel).ok(),
            note,
        };
        s
    }

    #[test]
    fn button_brightness_maps_cc_value_bands() {
        assert_eq!(button_brightness_from_value(1, true), Brightness::Dim);
        assert_eq!(button_brightness_from_value(50, true), Brightness::Normal);
        assert_eq!(button_brightness_from_value(127, true), Brightness::Bright);
    }

    #[test]
    fn button_brightness_zero_returns_ambient_when_on_else_off() {
        assert_eq!(
            button_brightness_from_value(0, true),
            BUTTON_BACKLIGHT_LEVEL
        );
        assert_eq!(button_brightness_from_value(0, false), Brightness::Off);
    }

    #[test]
    fn button_press_emits_cc_with_127() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::ButtonChanged {
                index: 22,
                pressed: true,
            },
            &Settings::default(),
            &rt(),
        );
        assert_eq!(bytes, Some([0xB0, 42, 127]));
    }

    #[test]
    fn button_release_emits_cc_with_0() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::ButtonChanged {
                index: 22,
                pressed: false,
            },
            &Settings::default(),
            &rt(),
        );
        assert_eq!(bytes, Some([0xB0, 42, 0]));
    }

    #[test]
    fn slider_moved_emits_cc() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::SliderMoved {
                raw: 100,
                cc_value: 63,
            },
            &Settings::default(),
            &rt(),
        );
        assert_eq!(bytes, Some([0xB0, 9, 63]));
    }

    #[test]
    fn slider_touch_disabled_drops_silently() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::SliderTouch { pressed: true },
            &Settings::default(),
            &rt(),
        );
        assert_eq!(bytes, None);
    }

    #[test]
    fn slider_touch_as_note_emits_note_on_off_with_configured_values() {
        let mut s = Settings::default();
        s.slider.touch = SliderTouchAction::Note {
            channel: None,
            note: 60,
            on_value: 100,
            off_value: 10,
        };

        let press = event_to_midi_bytes(&ControlEvent::SliderTouch { pressed: true }, &s, &rt());
        let release = event_to_midi_bytes(&ControlEvent::SliderTouch { pressed: false }, &s, &rt());
        assert_eq!(press, Some([0x90, 60, 100]));
        assert_eq!(release, Some([0x80, 60, 10]));
    }

    #[test]
    fn slider_touch_as_cc_emits_cc_with_on_off_values() {
        let mut s = Settings::default();
        s.slider.touch = SliderTouchAction::Cc {
            channel: None,
            cc: 70,
            on_value: 127,
            off_value: 0,
        };

        let press = event_to_midi_bytes(&ControlEvent::SliderTouch { pressed: true }, &s, &rt());
        let release = event_to_midi_bytes(&ControlEvent::SliderTouch { pressed: false }, &s, &rt());
        assert_eq!(press, Some([0xB0, 70, 127]));
        assert_eq!(release, Some([0xB0, 70, 0]));
    }

    #[test]
    fn pad_aftertouch_disabled_drops_silently() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::PadAftertouch {
                index: 0,
                pressure: 50,
            },
            &Settings::default(),
            &rt(),
        );
        assert_eq!(bytes, None);
    }

    #[test]
    fn pad_aftertouch_enabled_emits_poly_pressure_with_inherited_note() {
        let s = settings_with_pad_pressure_enabled(0, 0, None);
        let bytes = event_to_midi_bytes(
            &ControlEvent::PadAftertouch {
                index: 0,
                pressure: 100,
            },
            &s,
            &rt(),
        );
        // pads[0].hit.note default = 48
        assert_eq!(bytes, Some([0xA0, 48, 100]));
    }

    #[test]
    fn pad_aftertouch_enabled_respects_per_action_channel_and_note_override() {
        let s = settings_with_pad_pressure_enabled(0, 2, Some(60));
        let bytes = event_to_midi_bytes(
            &ControlEvent::PadAftertouch {
                index: 0,
                pressure: 100,
            },
            &s,
            &rt(),
        );
        assert_eq!(bytes, Some([0xA2, 60, 100]));
    }

    #[test]
    fn omitted_channel_defaults_to_channel_one_and_explicit_is_honored() {
        // Default button 22 omits a channel -> channel 0 (displayed 1).
        let bytes = event_to_midi_bytes(
            &ControlEvent::ButtonChanged {
                index: 22,
                pressed: true,
            },
            &Settings::default(),
            &rt(),
        );
        assert_eq!(bytes, Some([0xB0, 42, 127]));

        // An explicit per-action channel is still used.
        let mut s = Settings::default();
        s.buttons.0[22].press = ButtonPressAction::Cc {
            channel: MidiChannel::try_from(5).ok(),
            cc: 42,
        };
        let bytes = event_to_midi_bytes(
            &ControlEvent::ButtonChanged {
                index: 22,
                pressed: true,
            },
            &s,
            &rt(),
        );
        assert_eq!(bytes, Some([0xB5, 42, 127]));
    }

    #[test]
    fn pad_note_on_off_emit_note_messages() {
        let s = Settings::default();
        let on = event_to_midi_bytes(
            &ControlEvent::PadNoteOn {
                index: 0,
                velocity: 64,
            },
            &s,
            &rt(),
        );
        let off = event_to_midi_bytes(
            &ControlEvent::PadNoteOff {
                index: 0,
                velocity: 0,
            },
            &s,
            &rt(),
        );
        assert_eq!(on, Some([0x90, 48, 64]));
        assert_eq!(off, Some([0x80, 48, 0]));
    }

    // Unused imports are kept to allow re-introducing per-test if needed.
    #[allow(dead_code)]
    fn _ensure_imports_used() {
        let _ = PadHitAction::Note {
            channel: None,
            note: 0,
        };
        let _ = ButtonPressAction::Cc {
            channel: None,
            cc: 0,
        };
    }

    #[test]
    fn handle_event_drops_silently_when_dispatch_returns_none() {
        let mut backend = MidiBackend::with_sink(Settings::default(), CapturingSink::default());

        backend
            .handle_event(&ControlEvent::SliderTouch { pressed: true }, &rt())
            .unwrap();
        backend
            .handle_event(
                &ControlEvent::PadAftertouch {
                    index: 0,
                    pressure: 100,
                },
                &rt(),
            )
            .unwrap();

        assert!(
            backend.sink().sent.is_empty(),
            "got {:?}",
            backend.sink().sent
        );
    }

    #[test]
    fn off_actions_emit_nothing() {
        let mut s = Settings::default();
        s.encoder.turn = EncoderTurnAction::Off;
        s.slider.position = SliderPositionAction::Off;
        s.buttons.0[0].press = ButtonPressAction::Off;
        s.pads.0[0].hit = PadHitAction::Off;

        assert_eq!(
            event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: 1 }, &s, &rt()),
            None
        );
        assert_eq!(
            event_to_midi_bytes(
                &ControlEvent::SliderMoved {
                    raw: 100,
                    cc_value: 63,
                },
                &s,
                &rt(),
            ),
            None
        );
        assert_eq!(
            event_to_midi_bytes(
                &ControlEvent::ButtonChanged {
                    index: 0,
                    pressed: true,
                },
                &s,
                &rt(),
            ),
            None
        );
        assert_eq!(
            event_to_midi_bytes(
                &ControlEvent::PadNoteOn {
                    index: 0,
                    velocity: 64,
                },
                &s,
                &rt(),
            ),
            None
        );
    }

    #[test]
    fn handle_event_sends_button_press_under_default_settings() {
        let mut backend = MidiBackend::with_sink(Settings::default(), CapturingSink::default());
        backend
            .handle_event(
                &ControlEvent::ButtonChanged {
                    index: 22,
                    pressed: true,
                },
                &rt(),
            )
            .unwrap();
        assert_eq!(backend.sink().sent, vec![vec![0xB0, 42, 127]]);
    }

    #[test]
    fn step_absolute_advances_within_range() {
        let v = step_absolute(0, 1, 0, 127, 1, false);
        assert_eq!(v, 1);
        let v = step_absolute(v, 1, 0, 127, 1, false);
        assert_eq!(v, 2);
    }

    #[test]
    fn step_absolute_step3_scales_delta() {
        let v = step_absolute(10, 1, 0, 127, 3, false);
        assert_eq!(v, 13);
    }

    #[test]
    fn step_absolute_clamps_at_hi() {
        let v = step_absolute(10, 1, 0, 10, 1, false);
        assert_eq!(v, 10);
    }

    #[test]
    fn step_absolute_clamps_at_lo() {
        let v = step_absolute(0, -1, 0, 10, 1, false);
        assert_eq!(v, 0);
    }

    #[test]
    fn step_absolute_wraps_at_hi() {
        let v = step_absolute(10, 1, 0, 10, 1, true);
        assert_eq!(v, 0);
    }

    #[test]
    fn step_absolute_wraps_at_lo() {
        let v = step_absolute(0, -1, 0, 10, 1, true);
        assert_eq!(v, 10);
    }

    #[test]
    fn step_absolute_handles_multi_detent_delta() {
        let v = step_absolute(0, 2, 0, 127, 1, false);
        assert_eq!(v, 2);
    }

    #[test]
    fn encoder_relative_default_cw_emits_1() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::EncoderTurn { delta: 1 },
            &Settings::default(),
            &rt(),
        );
        assert_eq!(bytes, Some([0xB0, 1, 1]));
    }

    #[test]
    fn encoder_relative_default_ccw_emits_127() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::EncoderTurn { delta: -1 },
            &Settings::default(),
            &rt(),
        );
        assert_eq!(bytes, Some([0xB0, 1, 127]));
    }

    #[test]
    fn encoder_relative_step3_cw_emits_3() {
        let mut s = Settings::default();
        s.encoder.turn = EncoderTurnAction::Cc {
            channel: None,
            cc: 1,
            mode: CcValueMode::Relative { step: 3 },
        };
        let bytes = event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: 1 }, &s, &rt());
        assert_eq!(bytes, Some([0xB0, 1, 3]));
    }

    #[test]
    fn encoder_relative_step3_ccw_emits_125() {
        let mut s = Settings::default();
        s.encoder.turn = EncoderTurnAction::Cc {
            channel: None,
            cc: 1,
            mode: CcValueMode::Relative { step: 3 },
        };
        let bytes = event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: -1 }, &s, &rt());
        assert_eq!(bytes, Some([0xB0, 1, 125]));
    }

    #[test]
    fn encoder_relative_negative_step_reverses_direction() {
        let mut s = Settings::default();
        s.encoder.turn = EncoderTurnAction::Cc {
            channel: None,
            cc: 1,
            mode: CcValueMode::Relative { step: -3 },
        };
        // CW (delta +1) with a reversed step emits the CCW value, and vice versa.
        let cw = event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: 1 }, &s, &rt());
        assert_eq!(cw, Some([0xB0, 1, 125]));
        let ccw = event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: -1 }, &s, &rt());
        assert_eq!(ccw, Some([0xB0, 1, 3]));
    }

    #[test]
    fn encoder_absolute_negative_step_reverses_direction() {
        // From the default value (0) a CW turn with a -1 step decrements, so
        // without wrap it stays clamped at the low bound.
        let v = step_absolute(10, 1, 0, 127, -1, false);
        assert_eq!(v, 9);
        let v = step_absolute(10, -1, 0, 127, -1, false);
        assert_eq!(v, 11);
    }

    #[test]
    fn encoder_relative_multi_detent_scales() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::EncoderTurn { delta: 2 },
            &Settings::default(),
            &rt(),
        );
        assert_eq!(bytes, Some([0xB0, 1, 2]));
    }

    #[test]
    fn encoder_relative_offset_cw_emits_65() {
        let mut s = Settings::default();
        s.encoder.turn = EncoderTurnAction::Cc {
            channel: None,
            cc: 1,
            mode: CcValueMode::RelativeOffset { step: 1 },
        };
        let bytes = event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: 1 }, &s, &rt());
        assert_eq!(bytes, Some([0xB0, 1, 65]));
    }

    #[test]
    fn encoder_relative_offset_ccw_emits_63() {
        let mut s = Settings::default();
        s.encoder.turn = EncoderTurnAction::Cc {
            channel: None,
            cc: 1,
            mode: CcValueMode::RelativeOffset { step: 1 },
        };
        let bytes = event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: -1 }, &s, &rt());
        assert_eq!(bytes, Some([0xB0, 1, 63]));
    }

    #[test]
    fn encoder_relative_offset_step5_clamps_at_127() {
        let mut s = Settings::default();
        s.encoder.turn = EncoderTurnAction::Cc {
            channel: None,
            cc: 1,
            mode: CcValueMode::RelativeOffset { step: 5 },
        };
        let bytes = event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: 20 }, &s, &rt());
        assert_eq!(bytes, Some([0xB0, 1, 127]));
    }

    #[test]
    fn encoder_absolute_advances_counter_and_emits_value() {
        let mut s = Settings::default();
        s.encoder.turn = EncoderTurnAction::Cc {
            channel: None,
            cc: 1,
            mode: CcValueMode::Absolute {
                lo: 0,
                hi: 127,
                step: 1,
                wrap: false,
            },
        };
        let rt = crate::runtime_state::RuntimeState::default();
        let b1 = event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: 1 }, &s, &rt);
        let b2 = event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: 1 }, &s, &rt);
        let b3 = event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: 1 }, &s, &rt);
        assert_eq!(b1, Some([0xB0, 1, 1]));
        assert_eq!(b2, Some([0xB0, 1, 2]));
        assert_eq!(b3, Some([0xB0, 1, 3]));
        assert_eq!(rt.encoder_value(), 3);
    }

    #[test]
    fn encoder_absolute_clamps_at_hi() {
        let mut s = Settings::default();
        s.encoder.turn = EncoderTurnAction::Cc {
            channel: None,
            cc: 1,
            mode: CcValueMode::Absolute {
                lo: 0,
                hi: 10,
                step: 1,
                wrap: false,
            },
        };
        let rt = crate::runtime_state::RuntimeState::default();
        rt.set_encoder_value(10);
        let bytes = event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: 1 }, &s, &rt);
        assert_eq!(bytes, Some([0xB0, 1, 10]));
        assert_eq!(rt.encoder_value(), 10);
    }

    #[test]
    fn encoder_absolute_wraps_at_hi() {
        let mut s = Settings::default();
        s.encoder.turn = EncoderTurnAction::Cc {
            channel: None,
            cc: 1,
            mode: CcValueMode::Absolute {
                lo: 0,
                hi: 10,
                step: 1,
                wrap: true,
            },
        };
        let rt = crate::runtime_state::RuntimeState::default();
        rt.set_encoder_value(10);
        let bytes = event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: 1 }, &s, &rt);
        assert_eq!(bytes, Some([0xB0, 1, 0]));
        assert_eq!(rt.encoder_value(), 0);
    }

    #[test]
    fn encoder_absolute_synced_from_midi_in() {
        use crate::feedback::midi::apply_incoming_midi_message;
        use crate::outputs::DeviceOutputs;

        let mut s = Settings::default();
        s.encoder.turn = EncoderTurnAction::Cc {
            channel: None,
            cc: 1,
            mode: CcValueMode::Absolute {
                lo: 0,
                hi: 127,
                step: 1,
                wrap: false,
            },
        };
        let rt = crate::runtime_state::RuntimeState::default();
        let outputs = DeviceOutputs::new();

        // DAW echoes CC 1 with value 64 — sync to runtime state.
        apply_incoming_midi_message(&[0xB0, 1, 64], &outputs, &s, &rt);
        assert_eq!(rt.encoder_value(), 64);

        // Subsequent encoder turn moves relative to synced value.
        let bytes = event_to_midi_bytes(&ControlEvent::EncoderTurn { delta: -1 }, &s, &rt);
        assert_eq!(bytes, Some([0xB0, 1, 63]));
        assert_eq!(rt.encoder_value(), 63);
    }
}

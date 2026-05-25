use hidapi::HidResult;
use maschine_library::controls::Buttons;
use maschine_library::lights::{Brightness, PadColors};
use num::FromPrimitive;

use crate::events::ControlEvent;
use crate::outputs::DeviceOutputs;
use crate::settings::Settings;
use crate::settings::actions::{SliderLedMode, SliderLedSettings};

pub fn apply_local_output_feedback(
    outputs: &DeviceOutputs,
    settings: &Settings,
    event: &ControlEvent,
) -> HidResult<()> {
    match event {
        ControlEvent::SliderMoved { raw, .. } => {
            update_slider_lights(outputs, *raw, &settings.slider.led);
        }
        ControlEvent::PadNoteOn { index, .. } => {
            outputs.with_lights_mut(|lights| {
                lights.set_pad(*index, PadColors::Blue, Brightness::Normal);
            });
        }
        ControlEvent::PadNoteOff { index, .. } => {
            outputs.with_lights_mut(|lights| {
                lights.set_pad(*index, PadColors::Off, Brightness::Off);
            });
        }
        ControlEvent::ButtonChanged {
            index,
            pressed: false,
        } if settings.hardware.backlight_buttons
            && local_button_release_backlight_enabled(settings) =>
        {
            let brightness = settings.hardware.backlight_brightness.as_light_brightness();
            outputs.with_lights_mut(|lights| {
                if let Some(button) = Buttons::from_usize(*index)
                    && lights.button_has_light(button)
                {
                    lights.set_button(button, brightness);
                }
            });
        }
        _ => {}
    }

    Ok(())
}

fn local_button_release_backlight_enabled(settings: &Settings) -> bool {
    !settings.bridge.midi_bridge_virmidi
}

fn update_slider_lights(outputs: &DeviceOutputs, slider_raw: u8, led: &SliderLedSettings) {
    outputs.with_lights_mut(|lights| match led.mode {
        SliderLedMode::Bar => lights.render_slider_bar(slider_raw, led.color, led.stylized),
        SliderLedMode::Pan => lights.render_slider_pan(slider_raw, led.color, led.stylized),
        SliderLedMode::Dot => lights.render_slider_dot(slider_raw, led.color),
    });
}

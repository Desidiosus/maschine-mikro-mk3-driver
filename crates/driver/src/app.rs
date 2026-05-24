use hidapi::HidApi;
use maschine_library::controls::Buttons;
use maschine_library::hid::HidIo;
use maschine_library::lights::Brightness;
use maschine_library::lights::Lights;
use maschine_library::screen::{Screen, render_centered_text};
use maschine_library::{USB_PID, USB_VID};
use num::FromPrimitive;

use crate::backend::midi::MidiBackend;
use crate::error::{DriverError, DriverResult};
use crate::feedback::local::apply_local_output_feedback;
use crate::hid::{ControlState, decode_packet};
use crate::outputs::DeviceOutputs;
use crate::self_test::self_test;
use crate::settings::Settings;

pub fn run(settings: Settings) -> DriverResult<()> {
    let api = HidApi::new()?;
    let device = api.open(USB_VID, USB_PID)?;
    device.set_blocking_mode(false)?;
    run_with_device(settings, &device)
}

pub fn run_with_device<D: HidIo>(settings: Settings, device: &D) -> DriverResult<()> {
    settings.validate().map_err(DriverError::Settings)?;

    run_startup_self_test(device)?;

    let outputs = DeviceOutputs::new();
    prepare_startup_outputs(&outputs, &settings);
    outputs.flush(device)?;

    let mut backend = MidiBackend::new(&settings, &outputs)?;
    let mut state = ControlState::new();
    let mut buf = [0u8; 64];

    loop {
        buf.fill(0);
        let size = device.read_timeout(&mut buf, 1)?;

        if size < 1 {
            outputs.flush(device)?;
            continue;
        }

        for event in decode_packet(&mut state, &buf) {
            apply_local_output_feedback(&outputs, &settings, &event)?;
            backend.handle_event(&event)?;
        }

        outputs.flush(device)?;
    }
}

fn run_startup_self_test(device: &impl HidIo) -> DriverResult<()> {
    let mut screen = Screen::new();
    let mut lights = Lights::new();
    self_test(device, &mut screen, &mut lights)?;
    Ok(())
}

fn initialize_button_backlight(outputs: &DeviceOutputs, settings: &Settings) {
    if !settings.backlight_buttons {
        return;
    }

    let brightness = settings.backlight_brightness.as_light_brightness();

    outputs.with_lights_mut(|lights| {
        for idx in 0..41 {
            let Some(button) = Buttons::from_usize(idx) else {
                continue;
            };

            if lights.button_has_light(button) && lights.get_button(button) == Brightness::Off {
                lights.set_button(button, brightness);
            }
        }
    });
}

pub fn prepare_startup_outputs(outputs: &DeviceOutputs, settings: &Settings) {
    outputs.with_screen_mut(|screen| render_centered_text(screen, "MIDI MODE"));
    initialize_button_backlight(outputs, settings);
}

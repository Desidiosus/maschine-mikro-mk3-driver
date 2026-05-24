use hidapi::{HidApi, HidDevice};
use maschine_library::controls::Buttons;
use maschine_library::hid::HidIo;
use maschine_library::lights::Brightness;
use maschine_library::lights::Lights;
use maschine_library::preferences::{set_display_contrast, set_pad_sensitivity};
use maschine_library::screen::{Screen, render_centered_text};
use maschine_library::{USB_PID, USB_VID};
use num::FromPrimitive;

use crate::backend::midi::MidiBackend;
use crate::error::{DriverError, DriverResult};
use crate::feedback::local::apply_local_output_feedback;
use crate::hid::{ControlState, decode_packet_with_curve};
use crate::outputs::DeviceOutputs;
use crate::self_test::self_test;
use crate::settings::Settings;
use crate::soft_off::{SoftOffOutcome, SoftOffState, SoftOffSync};

pub fn run(settings: Settings) -> DriverResult<()> {
    let api = HidApi::new()?;
    let device = api.open(USB_VID, USB_PID)?;
    device.set_blocking_mode(false)?;
    apply_startup_preferences(&device, &settings)?;
    run_with_device(settings, &device)
}

fn apply_startup_preferences(device: &HidDevice, settings: &Settings) -> DriverResult<()> {
    set_pad_sensitivity(device, settings.hardware.pad_sensitivity)?;
    set_display_contrast(device, settings.hardware.display_contrast)?;
    Ok(())
}

pub fn run_with_device<D: HidIo>(settings: Settings, device: &D) -> DriverResult<()> {
    settings.validate().map_err(DriverError::Settings)?;
    let pad_velocity_curve = settings.hardware.pad_velocity_curve;

    run_startup_self_test(device)?;

    let outputs = DeviceOutputs::new();
    prepare_startup_outputs(&outputs, &settings);
    outputs.flush(device)?;

    let mut soft_off = SoftOffState::new(SoftOffSync::new());
    let mut backend = MidiBackend::new(&settings, &outputs, soft_off.sync())?;
    let mut state = ControlState::new();
    let mut buf = [0u8; 64];

    loop {
        buf.fill(0);
        let size = device.read_timeout(&mut buf, 1)?;

        if size < 1 {
            outputs.flush(device)?;
            continue;
        }

        for event in decode_packet_with_curve(&mut state, &buf, pad_velocity_curve) {
            if soft_off.observe_event(&outputs, &event) == SoftOffOutcome::Swallow {
                continue;
            }
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
    if !settings.hardware.backlight_buttons {
        return;
    }

    let brightness = settings.hardware.backlight_brightness.as_light_brightness();

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

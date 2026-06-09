use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hidapi::HidApi;
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
use crate::events::ControlEvent;
use crate::feedback::local::apply_local_output_feedback;
use crate::hid::{ControlState, decode_packet_with_curve};
use crate::outputs::DeviceOutputs;
use crate::self_test::self_test;
use crate::settings::Settings;
use crate::shared_settings::{SharedSettings, new_shared};
use crate::soft_off::{SoftOffOutcome, SoftOffState, SoftOffSync, blank_outputs};

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_shutdown_signal(_: libc::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
}

fn install_shutdown_signal_handlers() -> DriverResult<()> {
    unsafe {
        for sig in [libc::SIGINT, libc::SIGTERM] {
            if libc::signal(
                sig,
                handle_shutdown_signal as *const () as libc::sighandler_t,
            ) == libc::SIG_ERR
            {
                let err = std::io::Error::last_os_error();
                return Err(DriverError::Settings(format!(
                    "failed to install signal handler for signal {sig}: {err}"
                )));
            }
        }
    }
    Ok(())
}

pub fn run(settings: Settings) -> DriverResult<()> {
    let api = HidApi::new()?;
    let device = api.open(USB_VID, USB_PID)?;
    device.set_blocking_mode(false)?;
    let shared = new_shared(settings);
    apply_startup_preferences(&device, &shared.load())?;

    install_shutdown_signal_handlers()?;

    run_with_device(shared, &device, &SHUTDOWN_REQUESTED)
}

fn apply_startup_preferences<D: HidIo>(device: &D, settings: &Settings) -> DriverResult<()> {
    set_pad_sensitivity(device, settings.hardware.pad_sensitivity)?;
    set_display_contrast(device, settings.hardware.display_contrast)?;
    Ok(())
}

pub fn run_with_device<D: HidIo>(
    settings: SharedSettings,
    device: &D,
    shutdown_requested: &AtomicBool,
) -> DriverResult<()> {
    settings.load().validate().map_err(DriverError::Settings)?;

    run_startup_self_test(device)?;

    let outputs = DeviceOutputs::new();
    prepare_startup_outputs(&outputs, &settings.load());
    outputs.flush(device)?;

    let mut soft_off = SoftOffState::new(SoftOffSync::new());
    let soft_off_sync = soft_off.sync();
    let runtime_state = crate::runtime_state::RuntimeState::default();
    let mut backend =
        MidiBackend::new(&settings, &outputs, soft_off.sync(), runtime_state.clone())?;
    let mut state = ControlState::new();
    let mut buf = [0u8; 64];
    let mut slider_released_at: Option<Instant> = None;

    while !shutdown_requested.load(Ordering::Relaxed) {
        let snapshot = settings.load();
        let pad_velocity_curve = snapshot.hardware.pad_velocity_curve;
        let auto_off = snapshot.slider.led.auto_off_ms;
        let auto_off_color = snapshot.slider.led.color;

        buf.fill(0);
        let size = match device.read_timeout(&mut buf, 1) {
            Ok(s) => s,
            Err(err) => {
                if shutdown_requested.load(Ordering::Relaxed) {
                    break;
                }
                return Err(err.into());
            }
        };

        if size >= 1 {
            for event in decode_packet_with_curve(&mut state, &buf, pad_velocity_curve) {
                if soft_off.observe_event(&outputs, &event) == SoftOffOutcome::Swallow {
                    continue;
                }
                match &event {
                    ControlEvent::SliderTouch { pressed: false } => {
                        slider_released_at = Some(Instant::now());
                    }
                    ControlEvent::SliderTouch { pressed: true }
                    | ControlEvent::SliderMoved { .. } => {
                        slider_released_at = None;
                    }
                    _ => {}
                }
                apply_local_output_feedback(&outputs, &snapshot, &event)?;
                backend.handle_event(&event, &runtime_state)?;
            }
        }

        if let Some(released_at) = slider_released_at
            && auto_off > 0
            && released_at.elapsed() >= Duration::from_millis(auto_off)
            && !soft_off_sync.is_active()
        {
            outputs.with_lights_mut(|lights| {
                lights.render_slider_bar(0, auto_off_color, false);
            });
            slider_released_at = None;
        }

        outputs.flush(device)?;
    }

    blank_outputs(&outputs);
    outputs.flush(device)?;
    Ok(())
}

fn run_startup_self_test(device: &impl HidIo) -> DriverResult<()> {
    let mut screen = Screen::new();
    let mut lights = Lights::new();
    self_test(device, &mut screen, &mut lights)?;
    Ok(())
}

pub(crate) fn initialize_button_backlight(outputs: &DeviceOutputs, settings: &Settings) {
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

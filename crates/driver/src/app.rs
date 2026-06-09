use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};
use maschine_library::controls::Buttons;
use maschine_library::hid::HidIo;
use maschine_library::lights::Brightness;
use maschine_library::lights::Lights;
use maschine_library::preferences::{set_display_contrast, set_pad_sensitivity};
use maschine_library::screen::{Screen, render_centered_text};
use maschine_library::{USB_PID, USB_VID};
use num::FromPrimitive;
use protocol::{ControlRef, DriverToGui, MidiDir};

use crate::apply::{SideEffects, apply_side_effects};
use crate::backend::midi::MidiBackend;
use crate::error::{DriverError, DriverResult};
use crate::events::ControlEvent;
use crate::feedback::local::apply_local_output_feedback;
use crate::hid::{ControlState, decode_packet_with_curve};
use crate::ipc::{EventSubscriber, emit_event};
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

pub fn run(settings: Settings, config_path: std::path::PathBuf) -> DriverResult<()> {
    let shared = new_shared(settings);

    // Bind the IPC socket FIRST so the GUI can always connect and edit config,
    // even before/without a device. Settings applies + persistence work without
    // HID; only the runtime loop and hardware side effects need the device.
    let socket_path = protocol::socket_path().map_err(DriverError::Ipc)?;
    let (effects_tx, effects_rx) = mpsc::channel();
    let subscriber = crate::ipc::new_subscriber();
    let _ipc = crate::ipc::IpcServer::start(
        shared.clone(),
        config_path,
        effects_tx,
        subscriber.clone(),
        socket_path,
    )?;

    install_shutdown_signal_handlers()?;

    // Acquire the device, retrying so a later hotplug starts the runtime loop.
    // While waiting, the IPC server keeps serving config edits.
    loop {
        if SHUTDOWN_REQUESTED.load(Ordering::Relaxed) {
            return Ok(());
        }
        match open_device() {
            Ok(device) => {
                apply_startup_preferences(&device, &shared.load())?;
                // Discard any side effects queued by IPC applies while there was
                // no device — startup preferences above already pushed the
                // current settings to the freshly opened device.
                while effects_rx.try_recv().is_ok() {}
                return run_with_device(
                    shared,
                    &device,
                    &SHUTDOWN_REQUESTED,
                    effects_rx,
                    subscriber,
                );
            }
            Err(err) => {
                eprintln!(
                    "Maschine Mikro MK3 not available ({err}); IPC serving config, retrying…"
                );
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
    }
}

fn open_device() -> DriverResult<HidDevice> {
    let api = HidApi::new()?;
    let device = api.open(USB_VID, USB_PID)?;
    device.set_blocking_mode(false)?;
    Ok(device)
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
    effects_rx: Receiver<SideEffects>,
    subscriber: EventSubscriber,
) -> DriverResult<()> {
    settings.load().validate().map_err(DriverError::Settings)?;

    run_startup_self_test(device)?;

    let outputs = DeviceOutputs::new();
    prepare_startup_outputs(&outputs, &settings.load());
    outputs.flush(device)?;

    let mut soft_off = SoftOffState::new(SoftOffSync::new());
    let soft_off_sync = soft_off.sync();
    let runtime_state = crate::runtime_state::RuntimeState::default();
    let mut backend = MidiBackend::new(
        &settings,
        &outputs,
        soft_off.sync(),
        runtime_state.clone(),
        subscriber.clone(),
    )?;
    let mut state = ControlState::new();
    let mut buf = [0u8; 64];
    let mut slider_released_at: Option<Instant> = None;

    while !shutdown_requested.load(Ordering::Relaxed) {
        let snapshot = settings.load();

        // Apply any pending hardware side effects from IPC applies (HID is
        // owned by this thread).
        while let Ok(effects) = effects_rx.try_recv() {
            apply_side_effects(&effects, &snapshot, device, &outputs)?;
        }

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
                if let Some(control) = control_ref_for(&event) {
                    emit_event(&subscriber, DriverToGui::ControlActuated { control });
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
                if backend.handle_event(&event, &runtime_state)? {
                    emit_event(&subscriber, DriverToGui::MidiActivity { dir: MidiDir::Out });
                }
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

fn control_ref_for(event: &ControlEvent) -> Option<ControlRef> {
    match event {
        ControlEvent::ButtonChanged {
            index,
            pressed: true,
        } => Some(ControlRef::Button(*index as u8)),
        ControlEvent::PadNoteOn { index, .. } => Some(ControlRef::Pad(*index as u8)),
        ControlEvent::EncoderTurn { .. } => Some(ControlRef::Encoder),
        ControlEvent::SliderTouch { pressed: true } => Some(ControlRef::Slider),
        _ => None,
    }
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

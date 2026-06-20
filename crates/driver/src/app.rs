use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};
use maschine_library::controls::Buttons;
use maschine_library::hid::HidIo;
use maschine_library::lights::{BUTTON_BACKLIGHT_LEVEL, Brightness, Lights};
use maschine_library::preferences::{
    set_button_brightness, set_display_contrast, set_pad_sensitivity,
};
use maschine_library::screen::{Screen, render_centered_text};
use maschine_library::{USB_PID, USB_VID};
use num::FromPrimitive;
use protocol::{ControlRef, DriverToGui, MidiDir};

use crate::apply::{SideEffects, apply_device_registers, apply_output_overlays};
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

/// Why a device session ended.
enum SessionEnd {
    Shutdown,
    DeviceLost,
}

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

pub fn run(loaded: crate::settings::LoadedConfig) -> DriverResult<()> {
    let crate::settings::LoadedConfig {
        settings,
        persist_base,
        persist_path,
    } = loaded;
    let shared = new_shared(settings);
    let persist_base = std::sync::Arc::new(persist_base);

    // Bind the IPC socket FIRST so the GUI can always connect and edit config,
    // even before/without a device. Settings applies + persistence work without
    // HID; only the runtime loop and hardware side effects need the device.
    let socket_path = protocol::socket_path().map_err(DriverError::Ipc)?;
    let (effects_tx, effects_rx) = mpsc::channel();
    let subscriber = crate::ipc::new_subscriber();
    let device_present = std::sync::Arc::new(AtomicBool::new(false));
    let _ipc = crate::ipc::IpcServer::start(
        shared.clone(),
        persist_base,
        persist_path,
        effects_tx,
        subscriber.clone(),
        socket_path,
        device_present.clone(),
    )?;

    install_shutdown_signal_handlers()?;
    shared.load().validate().map_err(DriverError::Settings)?;

    // Persistent across device unplug/replug so MIDI ports stay stable.
    let outputs = DeviceOutputs::new();
    let mut soft_off = SoftOffState::new(SoftOffSync::new());
    let runtime_state = crate::runtime_state::RuntimeState::default();
    let mut backend = MidiBackend::new(
        &shared,
        &outputs,
        soft_off.sync(),
        runtime_state.clone(),
        subscriber.clone(),
    )?;

    // Acquire the device, retrying so a later hotplug starts the runtime loop.
    // On unplug the session ends with `DeviceLost` and we re-acquire without
    // tearing down the IPC server or the virtual MIDI ports.
    loop {
        if SHUTDOWN_REQUESTED.load(Ordering::Relaxed) {
            return Ok(());
        }
        match open_device() {
            Ok(device) => {
                if apply_startup_preferences(&device, &shared.load()).is_err() {
                    continue; // device flaked during init → re-acquire
                }
                // Discard any side effects queued by IPC applies while there was
                // no device — startup preferences above already pushed the
                // current settings to the freshly opened device.
                while effects_rx.try_recv().is_ok() {}
                device_present.store(true, Ordering::Release);
                emit_event(&subscriber, DriverToGui::DeviceConnected(true));

                let end = run_device_loop(
                    &shared,
                    &device,
                    &SHUTDOWN_REQUESTED,
                    &effects_rx,
                    &subscriber,
                    &mut soft_off,
                    &runtime_state,
                    &mut backend,
                    &outputs,
                );

                device_present.store(false, Ordering::Release);
                emit_event(&subscriber, DriverToGui::DeviceConnected(false));
                match end {
                    SessionEnd::Shutdown => return Ok(()),
                    SessionEnd::DeviceLost => {
                        eprintln!("Maschine Mikro MK3 disconnected; waiting for reconnect…");
                        continue;
                    }
                }
            }
            Err(_) => {
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
    set_button_brightness(device, settings.hardware.led_brightness)?;
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

    let outputs = DeviceOutputs::new();
    let mut soft_off = SoftOffState::new(SoftOffSync::new());
    let runtime_state = crate::runtime_state::RuntimeState::default();
    let mut backend = MidiBackend::new(
        &settings,
        &outputs,
        soft_off.sync(),
        runtime_state.clone(),
        subscriber.clone(),
    )?;
    let _ = run_device_loop(
        &settings,
        device,
        shutdown_requested,
        &effects_rx,
        &subscriber,
        &mut soft_off,
        &runtime_state,
        &mut backend,
        &outputs,
    );
    Ok(())
}

/// Run one device session. Returns when the device is lost (HID error) or a
/// shutdown is requested. Never propagates a HID error so the caller can
/// re-acquire the device without tearing down long-lived state.
#[allow(clippy::too_many_arguments)]
fn run_device_loop<D: HidIo>(
    settings: &SharedSettings,
    device: &D,
    shutdown_requested: &AtomicBool,
    effects_rx: &Receiver<SideEffects>,
    subscriber: &EventSubscriber,
    soft_off: &mut SoftOffState,
    runtime_state: &crate::runtime_state::RuntimeState,
    backend: &mut MidiBackend,
    outputs: &DeviceOutputs,
) -> SessionEnd {
    // Session setup; treat any HID error here as device loss.
    if run_startup_self_test(device).is_err() {
        return SessionEnd::DeviceLost;
    }
    prepare_startup_outputs(outputs, &settings.load());
    if outputs.flush(device).is_err() {
        return SessionEnd::DeviceLost;
    }

    let soft_off_sync = soft_off.sync();
    let mut state = ControlState::new();
    let mut buf = [0u8; 64];
    let mut slider_released_at: Option<Instant> = None;
    // Output overlays (backlight + pad idle LEDs) queued by IPC applies while
    // soft-off blanks the device. Accumulated here and flushed on wake against
    // the restored outputs, instead of lighting a sleeping device.
    let mut pending_overlays = SideEffects::default();
    // Tracks whether MIDI output is currently failing, so a broken port is logged
    // once (not once per event) and again only after it recovers.
    let mut midi_send_failed = false;

    while !shutdown_requested.load(Ordering::Relaxed) {
        // Apply any pending hardware side effects from IPC applies (HID is
        // owned by this thread). Re-load the settings per effect so the overlay
        // refresh reads the just-applied values.
        //
        // Device-register writes (sensitivity/contrast/brightness) apply
        // immediately — they are independent of the blanked display. Output
        // overlays (backlight + pad idle LEDs) would light a sleeping device, so
        // while soft-off is active they accumulate in `pending_overlays` and are
        // flushed on wake against the restored outputs.
        while let Ok(effects) = effects_rx.try_recv() {
            if apply_device_registers(&effects, device).is_err() {
                return SessionEnd::DeviceLost;
            }
            if soft_off_sync.is_active() {
                pending_overlays.refresh_backlight |= effects.refresh_backlight;
                pending_overlays.refresh_pad_leds |= effects.refresh_pad_leds;
            } else {
                apply_output_overlays(&effects, &settings.load(), outputs);
            }
        }
        if !soft_off_sync.is_active()
            && (pending_overlays.refresh_backlight || pending_overlays.refresh_pad_leds != 0)
        {
            apply_output_overlays(&pending_overlays, &settings.load(), outputs);
            pending_overlays = SideEffects::default();
        }

        // Captured after applying pending IPC changes so input decoding and local
        // feedback this iteration read the just-applied settings, not a snapshot
        // taken before the IPC thread stored them.
        let snapshot = settings.load();

        let pad_velocity_curve = snapshot.hardware.pad_velocity_curve;
        let auto_off = snapshot.slider.led.auto_off_ms;
        let auto_off_color = snapshot.slider.led.color;

        buf.fill(0);
        let size = match device.read_timeout(&mut buf, 1) {
            Ok(s) => s,
            Err(_) => {
                // A signal (SIGTERM/SIGINT) interrupts the read as EINTR → Err.
                // On shutdown, still blank the device before exiting (the device
                // is fine; only the read was interrupted).
                if shutdown_requested.load(Ordering::Relaxed) {
                    blank_outputs(outputs);
                    let _ = outputs.flush(device);
                    return SessionEnd::Shutdown;
                }
                return SessionEnd::DeviceLost; // unplugged → re-acquire
            }
        };

        if size >= 1 {
            for event in decode_packet_with_curve(&mut state, &buf, pad_velocity_curve) {
                if soft_off.observe_event(outputs, &event) == SoftOffOutcome::Swallow {
                    continue;
                }
                if let Some(control) = control_ref_for(&event) {
                    emit_event(subscriber, DriverToGui::ControlActuated { control });
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
                // Local feedback is best-effort: a failure here shouldn't end the
                // session (the flush below will catch real device loss).
                let _ = apply_local_output_feedback(outputs, &snapshot, &event);
                // A MIDI send failure shouldn't tear down the session (a transient
                // hiccup would needlessly drop the device); log it once and keep
                // going so a recovered port resumes without a restart.
                match backend.handle_event(&event, runtime_state) {
                    Ok(true) => {
                        midi_send_failed = false;
                        emit_event(subscriber, DriverToGui::MidiActivity { dir: MidiDir::Out });
                    }
                    Ok(false) => {}
                    Err(err) => {
                        if !midi_send_failed {
                            midi_send_failed = true;
                            eprintln!(
                                "MIDI send failed: {err} (continuing; suppressing further \
                                 errors until output recovers)"
                            );
                        }
                    }
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

        if outputs.flush(device).is_err() {
            return SessionEnd::DeviceLost;
        }
    }

    blank_outputs(outputs);
    let _ = outputs.flush(device); // device may already be gone on shutdown
    SessionEnd::Shutdown
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

fn for_each_backlit_button(outputs: &DeviceOutputs, mut f: impl FnMut(&mut Lights, Buttons)) {
    outputs.with_lights_mut(|lights| {
        for idx in 0..41 {
            let Some(button) = Buttons::from_usize(idx) else {
                continue;
            };
            if lights.button_has_light(button) {
                f(lights, button);
            }
        }
    });
}

pub(crate) fn initialize_button_backlight(outputs: &DeviceOutputs, settings: &Settings) {
    if settings.hardware.led_brightness == 0 {
        return;
    }
    for_each_backlit_button(outputs, |lights, button| {
        if lights.get_button(button) == Brightness::Off {
            lights.set_button(button, BUTTON_BACKLIGHT_LEVEL);
        }
    });
}

/// Re-apply the button-backlight setting to ALL backlight-capable buttons:
/// `BUTTON_BACKLIGHT_LEVEL` when brightness is non-zero, else `Off`. The
/// global `0xf3` preference scales the actual emitted intensity. This is a blunt
/// refresh — it may briefly override DAW-driven LED state until the next feedback
/// message, so it runs only when the backlight toggles on or off.
pub(crate) fn refresh_button_backlight(outputs: &DeviceOutputs, settings: &Settings) {
    let level = if settings.hardware.led_brightness > 0 {
        BUTTON_BACKLIGHT_LEVEL
    } else {
        Brightness::Off
    };
    for_each_backlit_button(outputs, |lights, button| lights.set_button(button, level));
}

/// Seed every pad LED to its active source's idle state, so Single/Dual-idle
/// pads glow at rest (not only after the first event). Source `Off`/`Velocity`
/// idle is dark.
pub(crate) fn initialize_pad_leds(outputs: &DeviceOutputs, settings: &Settings) {
    seed_pad_leds(outputs, settings, u16::MAX);
}

/// Re-seed only the pads whose bit is set in `mask` to their idle state, leaving
/// every other pad untouched so a config edit never clobbers a pad currently lit
/// by live feedback.
pub(crate) fn reseed_pad_leds(outputs: &DeviceOutputs, settings: &Settings, mask: u16) {
    seed_pad_leds(outputs, settings, mask);
}

fn seed_pad_leds(outputs: &DeviceOutputs, settings: &Settings, mask: u16) {
    outputs.with_lights_mut(|lights| {
        for index in 0..16 {
            if mask & (1 << index) == 0 {
                continue;
            }
            let (color, brightness) = settings.pads[index].led.resolve(false, 0);
            lights.set_pad(index, color, brightness);
        }
    });
}

pub fn prepare_startup_outputs(outputs: &DeviceOutputs, settings: &Settings) {
    outputs.with_screen_mut(|screen| render_centered_text(screen, "MIDI MODE"));
    initialize_button_backlight(outputs, settings);
    initialize_pad_leds(outputs, settings);
}

#[cfg(test)]
mod pad_led_seed_tests {
    use super::*;
    use maschine_library::lights::{Brightness, PadColors};
    use settings::{PadLedColorMode, PadLedSource, Settings};

    #[test]
    fn seeding_lights_single_idle_and_leaves_velocity_dark() {
        let outputs = DeviceOutputs::new();
        let mut settings = Settings::default();
        // pad 0: Single Green on the active (Out) source → dim idle.
        settings.pads[0].led.midi_out = PadLedColorMode::single(PadColors::Green);
        // pad 1: Velocity on the active source → dark idle.
        settings.pads[1].led.midi_out = PadLedColorMode::velocity();
        // pad 2: source Off → dark.
        settings.pads[2].led.source = PadLedSource::Off;

        initialize_pad_leds(&outputs, &settings);

        assert_eq!(
            outputs.with_lights(|l| l.get_pad(0)),
            (PadColors::Green, Brightness::Dim)
        );
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(1)),
            (PadColors::Off, Brightness::Off)
        );
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(2)),
            (PadColors::Off, Brightness::Off)
        );
    }

    #[test]
    fn reseed_only_touches_masked_pads() {
        let outputs = DeviceOutputs::new();
        let mut settings = Settings::default();
        settings.pads[0].led.midi_out = PadLedColorMode::single(PadColors::Green);
        // Pad 1 is currently lit by live feedback, not at its idle state.
        outputs.with_lights_mut(|l| l.set_pad(1, PadColors::Red, Brightness::Bright));

        // Re-seed only pad 0; pad 1 must be left untouched.
        reseed_pad_leds(&outputs, &settings, 1 << 0);

        assert_eq!(
            outputs.with_lights(|l| l.get_pad(0)),
            (PadColors::Green, Brightness::Dim)
        );
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(1)),
            (PadColors::Red, Brightness::Bright),
            "a targeted re-seed must not clobber an unrelated lit pad"
        );
    }
}

#[cfg(test)]
mod backlight_tests {
    use super::*;
    use maschine_library::controls::Buttons;
    use maschine_library::lights::Brightness;

    #[test]
    fn refresh_sets_all_backlit_buttons_to_ambient_then_off() {
        let outputs = DeviceOutputs::new();
        let mut settings = Settings::default();
        settings.hardware.led_brightness = 5;
        refresh_button_backlight(&outputs, &settings);
        outputs.with_lights_mut(|l| {
            if l.button_has_light(Buttons::Play) {
                assert_eq!(l.get_button(Buttons::Play), BUTTON_BACKLIGHT_LEVEL);
            }
        });

        settings.hardware.led_brightness = 0;
        refresh_button_backlight(&outputs, &settings);
        outputs.with_lights_mut(|l| {
            if l.button_has_light(Buttons::Play) {
                assert_eq!(l.get_button(Buttons::Play), Brightness::Off);
            }
        });
    }
}

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
use crate::paging::PagingAction;
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
    let write_lock = crate::settings::writer::new_write_lock();

    // Bind the IPC socket FIRST so the GUI can always connect and edit config,
    // even before/without a device. Settings applies + persistence work without
    // HID; only the runtime loop and hardware side effects need the device.
    let socket_path = protocol::socket_path().map_err(DriverError::Ipc)?;
    let (effects_tx, effects_rx) = mpsc::channel();
    let subscriber = crate::ipc::new_subscriber();
    let device_present = std::sync::Arc::new(AtomicBool::new(false));
    let (page_apply_tx, page_apply_join) = crate::settings::writer::spawn_page_apply_writer(
        shared.clone(),
        persist_base.clone(),
        persist_path.clone(),
        write_lock.clone(),
        subscriber.clone(),
    );
    let _ipc = crate::ipc::IpcServer::start(
        shared.clone(),
        persist_base,
        persist_path,
        effects_tx,
        subscriber.clone(),
        socket_path,
        device_present.clone(),
        write_lock,
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
    let result = (|| -> DriverResult<()> {
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
                        &page_apply_tx,
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
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
    })();

    // Close the channel so the writer drains any queued Commit, then wait for its
    // persist to finish — otherwise a page selected microseconds before SIGTERM is
    // lost when the process exits out from under the detached thread.
    drop(page_apply_tx);
    let _ = page_apply_join.join();
    result
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
    // Page applies need a consumer even here: without one the picker would
    // render and accept taps while the active page never moved.
    let (page_apply_tx, page_apply_join) =
        crate::settings::writer::spawn_live_page_applier(settings.clone(), subscriber.clone());
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
        &page_apply_tx,
    );
    drop(page_apply_tx);
    let _ = page_apply_join.join();
    Ok(())
}

/// Run one device session. Returns when the device is lost (HID error) or a
/// shutdown is requested. Never propagates a HID error so the caller can
/// re-acquire the device without tearing down long-lived state.
///
/// Wraps `run_device_session` so every exit path — shutdown, device loss, any
/// of the several `DeviceLost` returns inside the session — flushes held notes
/// exactly once on the way out. Soft-off and unplug both swallow or never
/// deliver the physical release, so without this a stale `held` entry would
/// survive into the next session on this backend.
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
    page_apply_tx: &std::sync::mpsc::Sender<crate::settings::writer::PageApplyMsg>,
) -> SessionEnd {
    let end = run_device_session(
        settings,
        device,
        shutdown_requested,
        effects_rx,
        subscriber,
        soft_off,
        runtime_state,
        backend,
        outputs,
        page_apply_tx,
    );
    let _ = backend.flush_held_notes();
    end
}

#[allow(clippy::too_many_arguments)]
fn run_device_session<D: HidIo>(
    settings: &SharedSettings,
    device: &D,
    shutdown_requested: &AtomicBool,
    effects_rx: &Receiver<SideEffects>,
    subscriber: &EventSubscriber,
    soft_off: &mut SoftOffState,
    runtime_state: &crate::runtime_state::RuntimeState,
    backend: &mut MidiBackend,
    outputs: &DeviceOutputs,
    page_apply_tx: &std::sync::mpsc::Sender<crate::settings::writer::PageApplyMsg>,
) -> SessionEnd {
    // A fresh session always starts awake. Soft-off state is long-lived (it
    // persists across unplug/replug so the MIDI ports stay stable), so a device
    // unplugged while asleep would otherwise resume with the shared soft-off flag
    // still set — silently dropping DAW feedback and deferring output overlays on
    // a device that looks alive. Once soft-off is disabled the wake combo is no
    // longer observed, so that phantom-sleep state would be unrecoverable.
    soft_off.force_wake(outputs);

    // Session setup; treat any HID error here as device loss.
    let startup = settings.load();
    if startup.driver.self_test_on_launch && run_startup_self_test(device).is_err() {
        return SessionEnd::DeviceLost;
    }
    prepare_startup_outputs(outputs, &startup);
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

    let mut paging = crate::paging::PagingState::new();
    // True once a Preview was sent this hold, so Group release commits exactly one
    // persist (and only when the page actually moved).
    let mut sent_preview = false;
    // Set whenever the picker's rendered content changes (open, page select) or
    // something else repainted the pads, so the picker is pushed once per change
    // instead of on every ~1 ms loop iteration.
    let mut picker_dirty = false;
    // Backstop repaint interval for the picker. `picker_dirty` covers our own
    // changes; this catches pad writes from the MIDI-in feedback thread, which
    // paints pad LEDs independently of this loop and would otherwise leave the
    // picker corrupted until the next select.
    const PICKER_REFRESH: Duration = Duration::from_millis(50);
    let mut picker_rendered_at: Option<Instant> = None;
    // Tracks soft-off state across iterations so sleep/wake edges (held-note
    // flush, picker repaint) fire exactly once per transition.
    let mut was_asleep = false;
    // Set when soft-off tears down an open picker at sleep, so the wake-side
    // reseed only fires when the restored snapshot could actually contain
    // picker colors — not on every wake (which would clobber live DAW LEDs).
    let mut picker_torn_down_at_sleep = false;

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
            if effects.wake_soft_off && soft_off_sync.is_active() {
                soft_off.force_wake(outputs);
            }
            if soft_off_sync.is_active() {
                pending_overlays.refresh_backlight |= effects.refresh_backlight;
                pending_overlays.refresh_pad_leds |= effects.refresh_pad_leds;
            } else {
                apply_output_overlays(&effects, &settings.load(), outputs);
                if effects.refresh_pad_leds != 0 && paging.is_picking() {
                    picker_dirty = true;
                }
            }
        }
        if !soft_off_sync.is_active()
            && (pending_overlays.refresh_backlight || pending_overlays.refresh_pad_leds != 0)
        {
            apply_output_overlays(&pending_overlays, &settings.load(), outputs);
            if pending_overlays.refresh_pad_leds != 0 && paging.is_picking() {
                picker_dirty = true;
            }
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
                let outcome = if snapshot.driver.soft_off_enabled {
                    soft_off.observe_event(outputs, &event)
                } else {
                    SoftOffOutcome::Forward
                };
                if outcome == SoftOffOutcome::Swallow {
                    // Soft-off owns the device while asleep; drop any latched
                    // picker hold so it can't resume hijacking the grid on wake.
                    // No repaint here: the device is blanked, so the grid is
                    // reseeded on the wake edge instead.
                    if soft_off_sync.is_active() && tear_down_picker(&mut paging, &mut sent_preview)
                    {
                        picker_torn_down_at_sleep = true;
                    }
                    continue;
                }
                if snapshot.pad_paging.enabled {
                    let active = snapshot.pad_paging.active;
                    let page_count = snapshot.pad_paging.pages.len();
                    match paging.observe_event(active, page_count, &event) {
                        PagingAction::None => {}
                        PagingAction::Swallow => {
                            if let ControlEvent::PadNoteOn { index, .. } = &event {
                                backend.mark_picker_tap(*index);
                            }
                            continue;
                        }
                        PagingAction::OpenPicker => {
                            picker_dirty = true;
                            continue;
                        }
                        PagingAction::SelectPage(target) => {
                            let _ = page_apply_tx
                                .send(crate::settings::writer::PageApplyMsg::Preview(target));
                            if let ControlEvent::PadNoteOn { index, .. } = &event {
                                backend.mark_picker_tap(*index);
                            }
                            sent_preview = true;
                            picker_dirty = true;
                            continue;
                        }
                        PagingAction::ClosePicker(final_page) => {
                            let pads = snapshot
                                .pad_paging
                                .pages
                                .get(final_page)
                                .map(|p| &p.pads)
                                .unwrap_or_else(|| snapshot.active_pads());
                            seed_pads_from(outputs, pads, u16::MAX);
                            if sent_preview {
                                let _ = page_apply_tx.send(
                                    crate::settings::writer::PageApplyMsg::Commit(final_page),
                                );
                                sent_preview = false;
                            }
                            continue;
                        }
                    }
                } else {
                    close_picker(outputs, &snapshot, &mut paging, &mut sent_preview);
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
                //
                // While the page picker is open it owns the 16 pad LEDs, so pad
                // feedback must not repaint them; non-pad feedback still applies.
                let pad_event = matches!(
                    event,
                    ControlEvent::PadNoteOn { .. }
                        | ControlEvent::PadNoteOff { .. }
                        | ControlEvent::PadAftertouch { .. }
                );
                if !(pad_event && paging.is_picking()) {
                    let _ = apply_local_output_feedback(outputs, &snapshot, &event);
                }
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

        let asleep = soft_off_sync.is_active();
        if !was_asleep && asleep {
            // Releases are swallowed while asleep; don't leave hung notes or
            // stale held state behind.
            let _ = backend.flush_held_notes();
        }
        if was_asleep && !asleep && picker_torn_down_at_sleep {
            // The restored soft-off snapshot still holds picker colors from
            // when the picker was open at sleep time, so the grid needs
            // repainting from `active_pads()` no matter what `pad_paging.enabled`
            // is now — the GUI may have toggled it off while asleep, and that
            // alone doesn't repaint the pads (`refresh_pad_leds` stays 0 in
            // `apply.rs`), so this reseed is the only thing that clears them.
            seed_pads_from(outputs, snapshot.active_pads(), u16::MAX);
            picker_torn_down_at_sleep = false;
        }
        was_asleep = asleep;

        if snapshot.pad_paging.enabled && paging.is_picking() && !soft_off_sync.is_active() {
            let stale = picker_rendered_at.is_none_or(|at| at.elapsed() >= PICKER_REFRESH);
            if picker_dirty || stale {
                let pending = paging
                    .pending_active()
                    .unwrap_or(snapshot.pad_paging.active);
                crate::paging::render_picker(outputs, &snapshot.pad_paging, pending);
                picker_dirty = false;
                picker_rendered_at = Some(Instant::now());
            }
        } else {
            picker_rendered_at = None;
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
    seed_pads_from(outputs, settings.active_pads(), mask);
}

fn seed_pads_from(outputs: &DeviceOutputs, pads: &crate::settings::PadsByIndex, mask: u16) {
    outputs.with_lights_mut(|lights| {
        for index in 0..16 {
            if mask & (1 << index) == 0 {
                continue;
            }
            let (color, brightness) = pads[index].led.resolve(false, 0);
            lights.set_pad(index, color, brightness);
        }
    });
}

/// Tear down an open page picker: drop the latched hold and forget any pending
/// commit. Returns whether a picker was actually open, which the callers use to
/// decide whether the grid still shows picker colors.
fn tear_down_picker(paging: &mut crate::paging::PagingState, sent_preview: &mut bool) -> bool {
    let was_picking = paging.is_picking();
    paging.reset();
    *sent_preview = false;
    was_picking
}

/// Tear the picker down and repaint the grid from the active page's idle LEDs.
/// Used on the transition where the picker's own close path never runs (paging
/// disabled mid-hold) so the grid is never left showing picker colors. Soft-off
/// entry deliberately does NOT call this — it must not reseed onto a blanked
/// device — and tears down without the repaint; see the `Swallow` branch above.
fn close_picker(
    outputs: &DeviceOutputs,
    settings: &Settings,
    paging: &mut crate::paging::PagingState,
    sent_preview: &mut bool,
) {
    if tear_down_picker(paging, sent_preview) {
        seed_pads_from(outputs, settings.active_pads(), u16::MAX);
    }
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
        settings.active_pads_mut()[0].led.midi_out = PadLedColorMode::single(PadColors::Green);
        // pad 1: Velocity on the active source → dark idle.
        settings.active_pads_mut()[1].led.midi_out = PadLedColorMode::velocity();
        // pad 2: source Off → dark.
        settings.active_pads_mut()[2].led.source = PadLedSource::Off;

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
        settings.active_pads_mut()[0].led.midi_out = PadLedColorMode::single(PadColors::Green);
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

    #[test]
    fn close_picker_only_reseeds_when_a_picker_was_actually_open() {
        let outputs = DeviceOutputs::new();
        let settings = Settings::default();
        // Pad 0 is currently lit by something other than its idle state.
        outputs.with_lights_mut(|l| l.set_pad(0, PadColors::Red, Brightness::Bright));

        // Not picking: close_picker must leave the grid untouched.
        let mut paging = crate::paging::PagingState::new();
        let mut sent_preview = false;
        close_picker(&outputs, &settings, &mut paging, &mut sent_preview);
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(0)),
            (PadColors::Red, Brightness::Bright),
            "close_picker must not repaint the grid when no picker was open"
        );

        // Picking: close_picker must reseed the grid and clear sent_preview.
        paging.observe_event(
            0,
            4,
            &ControlEvent::ButtonChanged {
                index: Buttons::Group as usize,
                pressed: true,
            },
        );
        assert!(paging.is_picking());
        sent_preview = true;

        close_picker(&outputs, &settings, &mut paging, &mut sent_preview);

        assert!(!paging.is_picking());
        assert!(!sent_preview);
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(0)),
            (PadColors::Off, Brightness::Off),
            "close_picker must repaint the grid from idle LEDs when a picker was open"
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

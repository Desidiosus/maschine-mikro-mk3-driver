use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant};

use maschine_library::controls::Buttons;
use maschine_library::hid::HidIo;
use maschine_library::lights::{BUTTON_BACKLIGHT_LEVEL, Brightness, Lights};
use maschine_library::screen::{Screen, render_centered_text};
use num::FromPrimitive;
use protocol::{ControlRef, DriverToGui, MidiDir};

use crate::apply::{SideEffects, apply_device_registers, apply_output_overlays};
use crate::backend::midi::MidiBackend;
use crate::device::{BorrowedController, DeviceController, HidController, RecoveryTuning};
use crate::error::{DriverError, DriverResult};
use crate::events::ControlEvent;
use crate::feedback::local::apply_local_output_feedback;
use crate::hid::{ControlState, decode_packet_with_curve};
use crate::ipc::{EventSubscriber, emit_event};
use crate::outputs::DeviceOutputs;
use crate::paging::PagingAction;
use crate::recovery::{RecoveryResult, recover_or_lost};
use crate::self_test::self_test;
use crate::session_observer::{NoopObserver, SessionObserver};
use crate::settings::Settings;
use crate::shared_settings::{SharedSettings, new_shared};
use crate::soft_off::{SoftOffOutcome, SoftOffState, SoftOffSync, blank_outputs};
use crate::write_watchdog::{self, WriteWatchdog};

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Pause before re-acquiring after a session ends. A controller wedged past
/// recovery still accepts `open` (feature-report control transfers survive an
/// OUT-endpoint wedge), so without it the loop would storm USB resets and
/// repaints and flap `DeviceConnected` at the GUI.
const REACQUIRE_BACKOFF: Duration = Duration::from_secs(2);

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

pub fn run(
    loaded: crate::settings::LoadedConfig,
    observer: &dyn SessionObserver,
) -> DriverResult<()> {
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

    // The controller holds the live settings handle so every reopen (recovery
    // or hotplug) re-applies the current settings, not a startup snapshot.
    let controller = HidController::new(shared.clone())?;
    let tuning = || RecoveryTuning::from(&shared.load().recovery);

    // Acquire the device, retrying so a later hotplug starts the runtime loop.
    // A session recovers a wedge in place and ends with `DeviceLost` only once
    // recovery gives up; we then re-acquire without tearing down the IPC server
    // or the virtual MIDI ports.
    let result = (|| -> DriverResult<()> {
        loop {
            if SHUTDOWN_REQUESTED.load(Ordering::Relaxed) {
                return Ok(());
            }
            match controller.open() {
                Ok(device) => {
                    // Clear soft-off sleep carried over from a previous session,
                    // or setup paints an awake display while the shared flag still
                    // suppresses DAW feedback and diverts overlay applies.
                    soft_off.reset_for_new_session();
                    // Discard any side effects queued by IPC applies while there was
                    // no device — `open` already pushed the current settings to the
                    // freshly opened device.
                    while effects_rx.try_recv().is_ok() {}
                    device_present.store(true, Ordering::Release);
                    emit_event(&subscriber, DriverToGui::DeviceConnected(true));

                    let end = run_device_loop(
                        &shared,
                        &controller,
                        device,
                        &SHUTDOWN_REQUESTED,
                        &effects_rx,
                        &subscriber,
                        &mut soft_off,
                        &runtime_state,
                        &mut backend,
                        &outputs,
                        &page_apply_tx,
                        &device_present,
                        &tuning,
                        observer,
                    );

                    // Recovery already emits `DeviceConnected(false)` when it drops
                    // the handle, so emit only if we still own the transition.
                    if device_present.swap(false, Ordering::AcqRel) {
                        emit_event(&subscriber, DriverToGui::DeviceConnected(false));
                    }
                    match end {
                        SessionEnd::Shutdown => return Ok(()),
                        SessionEnd::DeviceLost => {
                            eprintln!("Maschine Mikro MK3 disconnected; waiting for reconnect…");
                            thread::sleep(REACQUIRE_BACKOFF);
                            continue;
                        }
                    }
                }
                Err(_) => {
                    thread::sleep(REACQUIRE_BACKOFF);
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

/// Drive the loop against an already-open device. Reset is a no-op and reopen
/// always returns the same handle, so only `max_consecutive_recoveries` ends a
/// wedged session; setting it to `u32::MAX` would loop recovery forever.
pub fn run_with_device<D: HidIo>(
    settings: SharedSettings,
    device: &D,
    shutdown_requested: &AtomicBool,
    effects_rx: Receiver<SideEffects>,
    subscriber: EventSubscriber,
) -> DriverResult<()> {
    settings.load().validate().map_err(DriverError::Settings)?;

    let controller = BorrowedController::new(device);
    run_single_session(
        &settings,
        &controller,
        shutdown_requested,
        &effects_rx,
        &subscriber,
        &RecoveryTuning::default(),
    )
}

/// Run a single session that owns the device through `controller`, with no IPC
/// client and no live settings updates.
pub fn run_with_controller<C: DeviceController>(
    settings: Settings,
    controller: &C,
    shutdown_requested: &AtomicBool,
    tuning: &RecoveryTuning,
) -> DriverResult<()> {
    let shared = new_shared(settings);
    shared.load().validate().map_err(DriverError::Settings)?;

    let subscriber = crate::ipc::new_subscriber();
    let (_effects_tx, effects_rx) = mpsc::channel();
    run_single_session(
        &shared,
        controller,
        shutdown_requested,
        &effects_rx,
        &subscriber,
        tuning,
    )
}

/// Build the per-session scaffold (outputs, soft-off, runtime state, MIDI
/// backend) and run a single device session to completion.
fn run_single_session<C: DeviceController>(
    settings: &SharedSettings,
    controller: &C,
    shutdown_requested: &AtomicBool,
    effects_rx: &Receiver<SideEffects>,
    subscriber: &EventSubscriber,
    tuning: &RecoveryTuning,
) -> DriverResult<()> {
    let outputs = DeviceOutputs::new();
    let mut soft_off = SoftOffState::new(SoftOffSync::new());
    let runtime_state = crate::runtime_state::RuntimeState::default();
    let mut backend = MidiBackend::new(
        settings,
        &outputs,
        soft_off.sync(),
        runtime_state.clone(),
        subscriber.clone(),
    )?;
    // Page applies need a consumer even here: without one the picker would
    // render and accept taps while the active page never moved.
    let (page_apply_tx, page_apply_join) =
        crate::settings::writer::spawn_live_page_applier(settings.clone(), subscriber.clone());

    let device = controller.open()?;
    let device_present = AtomicBool::new(true);
    let tuning = || tuning.clone();
    let _ = run_device_loop(
        settings,
        controller,
        device,
        shutdown_requested,
        effects_rx,
        subscriber,
        &mut soft_off,
        &runtime_state,
        &mut backend,
        &outputs,
        &page_apply_tx,
        &device_present,
        &tuning,
        &NoopObserver,
    );
    drop(page_apply_tx);
    let _ = page_apply_join.join();
    Ok(())
}

/// Run one device session. HID errors are never propagated: recovery handles
/// them in place, and `DeviceLost` is returned only once recovery gives up, so
/// the caller can re-acquire.
///
/// Wraps `run_device_session` so every exit path — shutdown, device loss, any
/// of the several `DeviceLost` returns inside the session — flushes held notes
/// exactly once on the way out. Soft-off and unplug both swallow or never
/// deliver the physical release, so without this a stale `held` entry would
/// survive into the next session on this backend.
#[allow(clippy::too_many_arguments)]
fn run_device_loop<C: DeviceController>(
    settings: &SharedSettings,
    controller: &C,
    device: C::Device,
    shutdown_requested: &AtomicBool,
    effects_rx: &Receiver<SideEffects>,
    subscriber: &EventSubscriber,
    soft_off: &mut SoftOffState,
    runtime_state: &crate::runtime_state::RuntimeState,
    backend: &mut MidiBackend,
    outputs: &DeviceOutputs,
    page_apply_tx: &std::sync::mpsc::Sender<crate::settings::writer::PageApplyMsg>,
    device_present: &AtomicBool,
    tuning: &dyn Fn() -> RecoveryTuning,
    observer: &dyn SessionObserver,
) -> SessionEnd {
    // A wedged OUT endpoint blocks the runtime thread in `write()` for the
    // kernel's 5 s timeout, so a watchdog thread resets the device on its
    // behalf; the reset also completes the pending URB and unblocks the write.
    let session_tuning = tuning();
    let watchdog = WriteWatchdog::new(session_tuning.write_stall_timeout);
    let session = |watchdog: &WriteWatchdog| {
        run_device_session(
            settings,
            controller,
            device,
            shutdown_requested,
            effects_rx,
            subscriber,
            soft_off,
            runtime_state,
            backend,
            outputs,
            page_apply_tx,
            device_present,
            tuning,
            watchdog,
            observer,
        )
    };

    let reset = controller
        .out_of_band_reset()
        .filter(|_| session_tuning.enabled);
    let end = match reset {
        Some(reset) => thread::scope(|scope| {
            let watcher =
                write_watchdog::spawn(scope, &watchdog, reset, session_tuning.write_stall_poll);
            let end = session(&watchdog);
            watchdog.stop();
            let _ = watcher.join();
            end
        }),
        None => session(&watchdog),
    };

    let _ = backend.flush_held_notes();
    end
}

#[allow(clippy::too_many_arguments)]
fn run_device_session<C: DeviceController>(
    settings: &SharedSettings,
    controller: &C,
    mut device: C::Device,
    shutdown_requested: &AtomicBool,
    effects_rx: &Receiver<SideEffects>,
    subscriber: &EventSubscriber,
    soft_off: &mut SoftOffState,
    runtime_state: &crate::runtime_state::RuntimeState,
    backend: &mut MidiBackend,
    outputs: &DeviceOutputs,
    page_apply_tx: &std::sync::mpsc::Sender<crate::settings::writer::PageApplyMsg>,
    device_present: &AtomicBool,
    tuning: &dyn Fn() -> RecoveryTuning,
    watchdog: &WriteWatchdog,
    observer: &dyn SessionObserver,
) -> SessionEnd {
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
    // Consecutive read errors; a sustained streak escalates to USB-reset recovery.
    let mut read_error_streak: u32 = 0;
    // Cleared on each successful flush, so a device that reopens but stays
    // wedged eventually gives up to the re-acquire loop.
    let mut consecutive_recoveries: u32 = 0;

    // Returns from the enclosing session on `GaveUp` or `Shutdown`.
    macro_rules! recover {
        () => {{
            let tuning = tuning();
            if !tuning.enabled {
                eprintln!("USB auto-recovery disabled; reconnecting without a reset");
                return SessionEnd::DeviceLost;
            }
            // Park the watchdog so it can't reset underneath a reopen, and take
            // its flag so recovery knows whether the device was already reset.
            watchdog.disarm();
            let watchdog_already_reset = watchdog.take_reset_issued();
            let recovery_started = Instant::now();
            let outcome = recover_or_lost(
                device,
                controller,
                outputs,
                &mut state,
                soft_off,
                subscriber,
                device_present,
                shutdown_requested,
                &tuning,
                &mut consecutive_recoveries,
                watchdog_already_reset,
            );
            watchdog.arm();
            observer.recovery_finished(
                recovery_started.elapsed(),
                matches!(outcome, RecoveryResult::Recovered(_)),
            );
            device = match outcome {
                RecoveryResult::Recovered(dev) => dev,
                RecoveryResult::GaveUp => return SessionEnd::DeviceLost,
                RecoveryResult::Shutdown => return SessionEnd::Shutdown,
            };
            read_error_streak = 0;
        }};
    }

    // A shutdown requested mid-setup is observed by `recover!()` on failure or
    // by the main loop on success.
    loop {
        if session_setup_succeeded(&device, outputs, &settings.load()) {
            break;
        }
        observer.fault("device setup failed");
        recover!();
    }
    // Recoveries spent during setup must not count against the mid-session
    // budget, or a startup that needed a few resets would give up on the first
    // flush failure.
    consecutive_recoveries = 0;

    let mut paging = crate::paging::PagingState::new();
    let mut indicator = PageIndicator::new();
    let mut last_active = settings.load().pad_paging.active;
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
        let mut register_write_failed = false;
        while let Ok(effects) = effects_rx.try_recv() {
            if apply_device_registers(&effects, &device).is_err() {
                // A wedge can surface first on an IPC-driven register write,
                // so recover in place as for read and flush faults.
                register_write_failed = true;
                break;
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
        if register_write_failed {
            eprintln!("HID register write error; entering recovery");
            observer.fault("HID register write error");
            recover!();
            continue;
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
        match device.read_timeout(&mut buf, 1) {
            Ok(size) => {
                read_error_streak = 0;
                if size >= 1 {
                    for event in decode_packet_with_curve(&mut state, &buf, pad_velocity_curve) {
                        let outcome = if snapshot.driver.soft_off_enabled {
                            soft_off.observe_event(outputs, &event)
                        } else {
                            SoftOffOutcome::Forward
                        };
                        if outcome == SoftOffOutcome::Swallow {
                            // Soft-off owns the device while asleep; drop any
                            // latched picker hold so it can't resume hijacking the
                            // grid on wake. No repaint here: the device is blanked,
                            // so the grid is reseeded on the wake edge instead.
                            if soft_off_sync.is_active()
                                && tear_down_picker(&mut paging, &mut sent_preview)
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
                                    let _ = page_apply_tx.send(
                                        crate::settings::writer::PageApplyMsg::Preview(target),
                                    );
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
                                            crate::settings::writer::PageApplyMsg::Commit(
                                                final_page,
                                            ),
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
                        track_slider_release(&event, &mut slider_released_at);
                        // Local feedback is best-effort: a failure here shouldn't
                        // end the session (the flush below catches real device loss).
                        //
                        // While the page picker is open it owns the 16 pad LEDs, so
                        // pad feedback must not repaint them; non-pad feedback still
                        // applies.
                        let pad_event = matches!(
                            event,
                            ControlEvent::PadNoteOn { .. }
                                | ControlEvent::PadNoteOff { .. }
                                | ControlEvent::PadAftertouch { .. }
                        );
                        if !(pad_event && paging.is_picking()) {
                            let _ = apply_local_output_feedback(outputs, &snapshot, &event);
                        }
                        forward_event_to_backend(
                            backend,
                            &event,
                            runtime_state,
                            subscriber,
                            &mut midi_send_failed,
                        );
                    }
                }
            }
            Err(_) => {
                // A signal (SIGTERM/SIGINT) interrupts the read as EINTR → Err.
                // On shutdown, still blank the device before exiting (the device
                // is fine; only the read was interrupted).
                if shutdown_requested.load(Ordering::Relaxed) {
                    blank_outputs(outputs);
                    let _ = outputs.flush(&device);
                    return SessionEnd::Shutdown;
                }
                // Otherwise the controller may be wedged.
                read_error_streak += 1;
                let tuning = tuning();
                if read_error_streak > tuning.quick_retries {
                    eprintln!("HID read error (streak {read_error_streak}); entering recovery");
                    observer.fault("HID read error streak");
                    recover!();
                } else {
                    thread::sleep(tuning.quick_retry_delay);
                }
                continue;
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

        // Screen page indicator, driven by observing the active-page change from
        // any source (hardware writer thread or IPC). Never paint a soft-off
        // -blanked device, and never flash a fresh overlay while paging is
        // disabled — but keep ticking so an overlay already in flight still
        // expires even if paging is disabled mid-overlay.
        if !soft_off_sync.is_active() {
            let now = Instant::now();
            if snapshot.pad_paging.enabled {
                let current_active = snapshot.pad_paging.active;
                if current_active != last_active {
                    last_active = current_active;
                    // Same resolution the GUI shows, so a page never reads one
                    // way on the device screen and another in the app.
                    let label = snapshot.pad_paging.display_name(current_active);
                    indicator.show(outputs, &label, now);
                }
            } else {
                // Keep tracking the active page while paging is disabled so
                // re-enabling it later doesn't fire a stale indicator for a
                // change that happened while it was off.
                last_active = snapshot.pad_paging.active;
            }
            indicator.tick(outputs, now);
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

        // Bracket only the main-loop flush: setup sleeps between writes (the
        // self-test animation), which the watchdog would read as a stall.
        observer.flush_started(outputs.lights_dirty() || outputs.screen_dirty());
        watchdog.begin_write(watchdog.now_ms());
        let flushed = outputs.flush(&device);
        watchdog.end_write();
        observer.flush_finished(&flushed);

        match flushed {
            Ok(()) => {
                // The device is making progress, so the recovery budget restarts.
                consecutive_recoveries = 0;
            }
            Err(_) => {
                if shutdown_requested.load(Ordering::Relaxed) {
                    break;
                }
                // A flush error arrives only after the kernel USB timeout, so
                // retrying would burn more multi-second timeouts. Recovery's
                // dirty-all refresh repaints the dropped frame.
                eprintln!("HID flush error; entering recovery");
                recover!();
                continue;
            }
        }
    }

    blank_outputs(outputs);
    let _ = outputs.flush(&device); // device may already be gone on shutdown
    SessionEnd::Shutdown
}

/// Run the self-test and startup paint, returning `false` on the first I/O fault.
fn session_setup_succeeded<D: HidIo>(
    device: &D,
    outputs: &DeviceOutputs,
    settings: &Settings,
) -> bool {
    if settings.driver.self_test_on_launch && run_startup_self_test(device).is_err() {
        eprintln!("device setup failed (self-test); entering recovery");
        return false;
    }
    prepare_startup_outputs(outputs, settings);
    if outputs.flush(device).is_err() {
        eprintln!("device setup failed (startup paint); entering recovery");
        return false;
    }
    true
}

/// Stamp the slider release time so the idle timeout can later blank its LED.
fn track_slider_release(event: &ControlEvent, slider_released_at: &mut Option<Instant>) {
    match event {
        ControlEvent::SliderTouch { pressed: false } => {
            *slider_released_at = Some(Instant::now());
        }
        ControlEvent::SliderTouch { pressed: true } | ControlEvent::SliderMoved { .. } => {
            *slider_released_at = None;
        }
        _ => {}
    }
}

/// A send failure must not end the session, since a transient port hiccup would
/// needlessly drop the device. It is logged once and suppressed until a send
/// succeeds.
fn forward_event_to_backend(
    backend: &mut MidiBackend,
    event: &ControlEvent,
    runtime_state: &crate::runtime_state::RuntimeState,
    subscriber: &EventSubscriber,
    midi_send_failed: &mut bool,
) {
    match backend.handle_event(event, runtime_state) {
        Ok(true) => {
            *midi_send_failed = false;
            emit_event(subscriber, DriverToGui::MidiActivity { dir: MidiDir::Out });
        }
        Ok(false) => {}
        Err(err) => {
            if !*midi_send_failed {
                *midi_send_failed = true;
                eprintln!(
                    "MIDI send failed: {err} (continuing; suppressing \
                     further errors until output recovers)"
                );
            }
        }
    }
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

/// A brief on-screen page-name overlay, restored to the prior screen after a
/// short delay. Snapshots the screen only when a fresh overlay starts, so
/// back-to-back page changes during a Group sweep extend the deadline without
/// capturing the overlay itself.
struct PageIndicator {
    snapshot: Option<Screen>,
    /// Exactly what the overlay drew. On expiry the previous screen is restored
    /// only if the live screen still matches this — otherwise the DAW (or any
    /// other writer) has taken the screen since, and its content wins.
    rendered: Option<Screen>,
    until: Option<Instant>,
}

impl PageIndicator {
    const DURATION: Duration = Duration::from_millis(800);

    fn new() -> Self {
        Self {
            snapshot: None,
            rendered: None,
            until: None,
        }
    }

    fn show(&mut self, outputs: &DeviceOutputs, label: &str, now: Instant) {
        // One lock hold: snapshot, draw, and capture what we drew, so a
        // concurrent DAW write can't land between the draw and the capture and
        // poison the comparison `tick` later relies on.
        outputs.with_screen_mut(|screen| {
            if self.until.is_none() {
                self.snapshot = Some(screen.clone());
            }
            render_centered_text(screen, label);
            self.rendered = Some(screen.clone());
        });
        self.until = Some(now + Self::DURATION);
    }

    fn tick(&mut self, outputs: &DeviceOutputs, now: Instant) {
        if let Some(deadline) = self.until
            && now >= deadline
        {
            // Compare and restore under one lock hold, so a write can't slip in
            // after the check and get clobbered by the restore.
            outputs.with_screen_mut(|live| {
                let untouched = self.rendered.as_ref().is_some_and(|drawn| live == drawn);
                if untouched && let Some(screen) = self.snapshot.take() {
                    *live = screen;
                }
            });
            // Discard the snapshot if it wasn't consumed above: someone else owns
            // the screen now and must not be reverted by a later tick.
            self.snapshot = None;
            self.rendered = None;
            self.until = None;
        }
    }
}

#[cfg(test)]
mod page_indicator_tests {
    use super::*;

    #[test]
    fn restore_is_skipped_when_something_else_wrote_the_screen() {
        let outputs = DeviceOutputs::new();
        outputs.with_screen_mut(|s| render_centered_text(s, "MIDI MODE"));
        let original = outputs.with_screen(Clone::clone);

        let mut indicator = PageIndicator::new();
        let t0 = Instant::now();
        indicator.show(&outputs, "Page 2", t0);

        // The DAW writes the screen while the overlay is up (midir thread).
        outputs.with_screen_mut(|s| render_centered_text(s, "DAW TEXT"));
        let daw = outputs.with_screen(Clone::clone);

        indicator.tick(&outputs, t0 + PageIndicator::DURATION);

        assert_eq!(
            outputs.with_screen(Clone::clone),
            daw,
            "a concurrent DAW screen write must not be reverted by the indicator"
        );
        assert_ne!(outputs.with_screen(Clone::clone), original);
    }

    #[test]
    fn restore_happens_when_the_overlay_is_untouched() {
        let outputs = DeviceOutputs::new();
        outputs.with_screen_mut(|s| render_centered_text(s, "MIDI MODE"));
        let original = outputs.with_screen(Clone::clone);

        let mut indicator = PageIndicator::new();
        let t0 = Instant::now();
        indicator.show(&outputs, "Page 2", t0);
        indicator.tick(&outputs, t0 + PageIndicator::DURATION);

        assert_eq!(
            outputs.with_screen(Clone::clone),
            original,
            "an untouched overlay restores the previous screen"
        );
    }

    #[test]
    fn tick_expires_the_overlay_even_without_a_follow_up_show() {
        // Simulates paging being disabled mid-overlay: the run loop stops
        // calling `show` (no page changes are being observed), but must keep
        // calling `tick` so an in-flight overlay still expires instead of
        // sticking on the physical screen forever.
        let outputs = DeviceOutputs::new();
        outputs.with_screen_mut(|s| render_centered_text(s, "MIDI MODE"));
        let original = outputs.with_screen(Clone::clone);

        let mut indicator = PageIndicator::new();
        let t0 = Instant::now();
        indicator.show(&outputs, "Page 2", t0);

        indicator.tick(&outputs, t0 + PageIndicator::DURATION / 2);
        assert_ne!(
            outputs.with_screen(Clone::clone),
            original,
            "overlay should still be showing before the deadline"
        );

        indicator.tick(&outputs, t0 + PageIndicator::DURATION);
        assert_eq!(
            outputs.with_screen(Clone::clone),
            original,
            "ticking past the deadline restores the pre-overlay screen even \
             without another show()"
        );
    }
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

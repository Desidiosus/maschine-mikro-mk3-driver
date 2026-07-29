pub mod local;
pub mod midi;

use crate::outputs::DeviceOutputs;
use crate::settings::{PadLedSource, Settings};

/// Render pad `index`'s LED from the color mode bound to `expected`, but only
/// when the pad's configured source matches — so the In and Out feedback paths
/// never fight over the same LED. `on` is the note-on/hit state; `velocity`
/// feeds the `Velocity` color mode.
pub(crate) fn render_pad_led(
    outputs: &DeviceOutputs,
    settings: &Settings,
    expected: PadLedSource,
    index: usize,
    on: bool,
    velocity: u8,
) {
    let led = &settings.active_pads()[index].led;
    if led.source != expected {
        return;
    }
    let (color, brightness) = led.resolve(on, velocity);
    outputs.with_lights_mut(|lights| lights.set_pad(index, color, brightness));
}

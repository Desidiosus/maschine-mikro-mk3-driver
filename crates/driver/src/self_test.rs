use hidapi::HidResult;
use maschine_library::hid::HidIo;
use maschine_library::lights::{Brightness, Lights, PadColors};
use maschine_library::screen::{SCREEN_HEIGHT, Screen, render_centered_text_with_style};
use std::{thread, time};

fn prepare_self_test_splash(screen: &mut Screen) {
    let scale = 2;
    let y = (SCREEN_HEIGHT - (8 * scale)) / 2;
    render_centered_text_with_style(screen, "MASCHINE", scale, y);
}

fn self_test_pad_colors() -> [PadColors; 16] {
    [
        PadColors::Orange,
        PadColors::LightOrange,
        PadColors::WarmYellow,
        PadColors::Yellow,
        PadColors::Lime,
        PadColors::Green,
        PadColors::Mint,
        PadColors::Cyan,
        PadColors::Turquoise,
        PadColors::Blue,
        PadColors::Plum,
        PadColors::Violet,
        PadColors::Purple,
        PadColors::Magenta,
        PadColors::Fuchsia,
        PadColors::White,
    ]
}

pub(crate) fn self_test(
    device: &impl HidIo,
    screen: &mut Screen,
    lights: &mut Lights,
) -> HidResult<()> {
    prepare_self_test_splash(screen);
    screen.write(device)?;
    thread::sleep(time::Duration::from_millis(1000));

    for i in 0..39 {
        lights.set_button(num::FromPrimitive::from_u32(i).unwrap(), Brightness::Bright);
        lights.write(device)?;
        lights.set_button(num::FromPrimitive::from_u32(i).unwrap(), Brightness::Normal);
        lights.write(device)?;
        lights.set_button(num::FromPrimitive::from_u32(i).unwrap(), Brightness::Dim);
        lights.write(device)?;
    }

    for (i, color) in self_test_pad_colors().into_iter().enumerate() {
        lights.set_pad(i, color, Brightness::Bright);
        lights.write(device)?;
        lights.set_pad(i, color, Brightness::Normal);
        lights.write(device)?;
        lights.set_pad(i, color, Brightness::Dim);
        lights.write(device)?;
    }

    for i in 0..25 {
        lights.set_slider(i, PadColors::White, Brightness::Bright);
        lights.write(device)?;
        lights.set_slider(i, PadColors::White, Brightness::Normal);
        lights.write(device)?;
        lights.set_slider(i, PadColors::White, Brightness::Dim);
        lights.write(device)?;
    }

    lights.reset();
    lights.write(device)?;
    screen.reset();
    screen.write(device)?;

    Ok(())
}

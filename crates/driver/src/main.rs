use clap::Parser;
use driver::app;
use driver::error::DriverResult;
use hidapi::HidResult;
use maschine_library::font::Font;
use maschine_library::hid::HidIo;
use maschine_library::screen::{
    SCREEN_TEXT_CHAR_WIDTH, SCREEN_TEXT_SCALE, SCREEN_TEXT_Y_POSITION, SCREEN_WIDTH, Screen,
    render_centered_text,
};
use maschine_library::{USB_PID, USB_VID};
use std::process::ExitCode;
use std::{thread, time::Duration};

#[derive(Parser, Debug)]
#[clap(
    name = "Maschine Mikro MK3 Userspace MIDI driver",
    version = env!("CARGO_PKG_VERSION"),
    author = env!("CARGO_PKG_AUTHORS"),
)]
struct Args {
    #[clap(
        short,
        long,
        help = "Config file (see default_config.toml for available keys)"
    )]
    config: Option<String>,

    #[clap(short, long, help = "Print text on screen (slides if > 4 chars)")]
    text: Option<String>,

    #[clap(
        short,
        long,
        help = "Issue a USB reset to the device and exit (recover a wedged controller)"
    )]
    reset: bool,
}

fn display_text(device: &impl HidIo, screen: &mut Screen, text: &str) -> HidResult<()> {
    if text.chars().count() <= 4 {
        render_centered_text(screen, text);
        screen.write(device)?;

        println!("Displaying text: {}", text);
        thread::sleep(Duration::from_secs(3));
    } else {
        let text_width = text.chars().count() * SCREEN_TEXT_CHAR_WIDTH;
        let total_distance = SCREEN_WIDTH + text_width;

        println!("Sliding text: {}", text);

        for offset in 0..total_distance {
            screen.reset();
            let x_pos = SCREEN_WIDTH as i32 - offset as i32;

            for (i, ch) in text.chars().enumerate() {
                let char_x = x_pos + (i * SCREEN_TEXT_CHAR_WIDTH) as i32;
                if char_x >= -(SCREEN_TEXT_CHAR_WIDTH as i32)
                    && char_x < SCREEN_WIDTH as i32
                    && char_x >= 0
                {
                    Font::write_char(
                        screen,
                        SCREEN_TEXT_Y_POSITION,
                        char_x as usize,
                        ch,
                        SCREEN_TEXT_SCALE,
                    );
                }
            }

            screen.write(device)?;
            thread::sleep(Duration::from_millis(30));
        }
    }

    Ok(())
}

/// After a USB reset the first open can race re-enumeration, so retry with a
/// settle between attempts. Returns the last open error once attempts run out.
fn reopen_with_retry<C: driver::device::DeviceController>(
    controller: &C,
    tuning: &driver::device::RecoveryTuning,
) -> DriverResult<C::Device> {
    let mut last_err = None;
    for attempt in 1..=tuning.max_reopen_attempts {
        match controller.open() {
            Ok(device) => return Ok(device),
            Err(err) => {
                eprintln!(
                    "reopen attempt {attempt}/{} failed: {err}",
                    tuning.max_reopen_attempts
                );
                last_err = Some(err);
                thread::sleep(tuning.reset_settle);
            }
        }
    }
    Err(last_err.expect("at least one open attempt was made"))
}

fn run() -> DriverResult<()> {
    let args = Args::parse();

    if args.reset {
        use driver::device::{DeviceController, HidController, RecoveryTuning};
        use driver::shared_settings::new_shared;

        let tuning = RecoveryTuning::default();
        // Re-init with the user's saved config so a recovered device keeps their
        // hardware preferences, falling back to defaults if it can't be loaded.
        let settings = driver::settings::load_config(args.config.as_deref())
            .map(|loaded| loaded.settings)
            .unwrap_or_default();
        let controller = HidController::new(new_shared(settings))?;
        // Issue resets best-effort: a wedged device may briefly drop off the bus,
        // so one failed reset must not abort the recovery before the reopen.
        for i in 1..=tuning.reset_attempts {
            match controller.reset() {
                Ok(()) => println!(
                    "USB reset {i}/{} issued to {USB_VID:#06x}:{USB_PID:#06x}",
                    tuning.reset_attempts
                ),
                Err(err) => eprintln!("USB reset {i}/{} failed: {err}", tuning.reset_attempts),
            }
            thread::sleep(tuning.reset_settle);
        }
        // Reopen to re-init the device (contrast, pad sensitivity) and paint a
        // confirmation so a recovered display is visibly alive.
        let device = reopen_with_retry(&controller, &tuning)?;
        let mut screen = Screen::new();
        render_centered_text(&mut screen, "OK");
        screen.write(&device)?;
        println!("device reset and reinitialized");
        return Ok(());
    }

    if let Some(text) = args.text {
        let api = hidapi::HidApi::new()?;
        let device = api.open(USB_VID, USB_PID)?;
        device.set_blocking_mode(false)?;

        let mut screen = Screen::new();
        display_text(&device, &mut screen, &text)?;

        screen.reset();
        screen.write(&device)?;
        return Ok(());
    }

    let loaded = driver::settings::load_config(args.config.as_deref())
        .map_err(driver::error::DriverError::Settings)?;
    loaded
        .settings
        .validate()
        .map_err(driver::error::DriverError::Settings)?;

    println!("Running with settings:");
    println!("{:?}", loaded.settings);

    app::run(loaded)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

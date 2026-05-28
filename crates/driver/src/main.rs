use clap::Parser;
use driver::app;
use driver::error::DriverResult;
use driver::settings::Settings;
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

fn run() -> DriverResult<()> {
    let args = Args::parse();

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

    let mut builder = config::Config::builder();

    if let Some(config_fn) = args.config {
        builder = builder.add_source(config::File::with_name(config_fn.as_str()));
    }

    let partial: driver::settings::PartialSettings = builder
        .build()
        .map_err(|err| driver::error::DriverError::Settings(err.to_string()))?
        .try_deserialize()
        .map_err(|err| driver::error::DriverError::Settings(err.to_string()))?;
    let settings = Settings::default().merge_overrides(partial);
    settings
        .validate()
        .map_err(driver::error::DriverError::Settings)?;

    println!("Running with settings:");
    println!("{settings:?}");

    app::run(settings)
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

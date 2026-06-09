use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use driver::apply::SideEffects;
use driver::ipc::{IpcServer, new_subscriber};
use driver::settings::Settings;
use driver::settings::persist::load_xdg;
use driver::shared_settings::new_shared;
use protocol::frame::{read_frame, write_frame};
use protocol::{DriverToGui, GuiToDriver};
use settings::PartialSettings;

fn unique(stem: &str, ext: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("mmk3-ipc-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{stem}-{}.{ext}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

struct Harness {
    server: Option<IpcServer>,
    handle: driver::shared_settings::SharedSettings,
    config: PathBuf,
    effects_rx: mpsc::Receiver<SideEffects>,
    client: UnixStream,
    reader: BufReader<UnixStream>,
}

fn start(name: &str) -> Harness {
    let handle = new_shared(Settings::default());
    let config = unique(name, "toml");
    let sock = unique(name, "sock");
    let (effects_tx, effects_rx) = mpsc::channel();
    let subscriber = new_subscriber();
    let server = IpcServer::start(
        handle.clone(),
        config.clone(),
        effects_tx,
        subscriber,
        sock.clone(),
    )
    .unwrap();
    let client = UnixStream::connect(&sock).unwrap();
    let reader = BufReader::new(client.try_clone().unwrap());
    Harness {
        server: Some(server),
        handle,
        config,
        effects_rx,
        client,
        reader,
    }
}

#[test]
fn get_settings_returns_a_snapshot() {
    let mut h = start("get");
    write_frame(&mut h.client, &GuiToDriver::GetSettings).unwrap();
    let resp: DriverToGui = read_frame(&mut h.reader).unwrap().unwrap();
    assert!(matches!(resp, DriverToGui::Settings(_)));
    h.server.take();
}

#[test]
fn apply_acks_persists_updates_and_routes_side_effects() {
    let mut h = start("apply");
    let delta: PartialSettings = toml::from_str("[hardware]\npad_sensitivity = 80\n").unwrap();
    write_frame(
        &mut h.client,
        &GuiToDriver::Apply {
            seq: 7,
            delta: Box::new(delta),
        },
    )
    .unwrap();

    let ack: DriverToGui = read_frame(&mut h.reader).unwrap().unwrap();
    assert!(matches!(
        ack,
        DriverToGui::Ack {
            seq: 7,
            result: Ok(())
        }
    ));
    let snap: DriverToGui = read_frame(&mut h.reader).unwrap().unwrap();
    assert!(matches!(snap, DriverToGui::Settings(_)));

    assert_eq!(h.handle.load().hardware.pad_sensitivity, 80);
    let effects = h
        .effects_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("side effects routed to loop");
    assert_eq!(effects.pad_sensitivity, Some(80));

    let reloaded = load_xdg(&h.config).unwrap();
    assert_eq!(reloaded.hardware.pad_sensitivity, 80);
    h.server.take();
}

#[test]
fn invalid_apply_acks_err_without_state_change() {
    let mut h = start("invalid");
    let delta: PartialSettings = toml::from_str("[hardware]\npad_sensitivity = 200\n").unwrap();
    write_frame(
        &mut h.client,
        &GuiToDriver::Apply {
            seq: 1,
            delta: Box::new(delta),
        },
    )
    .unwrap();

    let ack: DriverToGui = read_frame(&mut h.reader).unwrap().unwrap();
    assert!(matches!(
        ack,
        DriverToGui::Ack {
            seq: 1,
            result: Err(_)
        }
    ));
    assert_eq!(h.handle.load().hardware.pad_sensitivity, 50);
    assert!(!h.config.exists());
    h.server.take();
}

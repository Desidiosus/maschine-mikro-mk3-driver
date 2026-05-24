use std::path::Path;

fn test_settings() -> driver::settings::Settings {
    driver::settings::Settings {
        client_name: "Client".into(),
        port_name: "Port".into(),
        port_name_in: "Input".into(),
        autoconnect_virmidi: false,
        ..driver::settings::Settings::default()
    }
}

#[test]
fn runtime_constructor_creates_midi_backend_when_seq_available() {
    if !Path::new("/dev/snd/seq").exists() {
        return;
    }

    let outputs = driver::outputs::DeviceOutputs::new();
    let soft_off = driver::soft_off::SoftOffSync::new();
    driver::backend::midi::MidiBackend::new(&test_settings(), &outputs, soft_off).unwrap();
}

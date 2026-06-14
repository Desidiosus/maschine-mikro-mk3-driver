use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use driver::settings::persist::load_config_with_xdg;
use driver::settings::{Settings, load_xdg};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "mkk3-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn startup_creates_loadable_stub_when_xdg_missing() {
    let dir = TempDir::new();
    let xdg_path = dir.path().join("config.toml");
    assert!(!xdg_path.exists());

    let settings = load_xdg(&xdg_path).unwrap();

    assert!(
        xdg_path.exists(),
        "a config file should be written on first run"
    );
    assert_eq!(settings, Settings::default());

    // Second startup must load the just-written stub without error and still
    // resolve to defaults (the stub holds no overrides).
    let reloaded = load_xdg(&xdg_path).unwrap();
    assert_eq!(reloaded, Settings::default());
}

#[test]
fn existing_xdg_file_loads_and_merges() {
    let dir = TempDir::new();
    let xdg_path = dir.path().join("config.toml");
    fs::write(
        &xdg_path,
        r#"
[hardware]
pad_sensitivity = 88
display_contrast = 12
pad_velocity_curve = "hard2"
"#,
    )
    .unwrap();

    let settings = load_xdg(&xdg_path).unwrap();

    assert_eq!(settings.hardware.pad_sensitivity, 88);
    assert_eq!(settings.hardware.display_contrast, 12);
    assert_eq!(
        format!("{:?}", settings.hardware.pad_velocity_curve),
        "Hard2",
        "overridden field is applied"
    );
    // A value we did not override stays at its default.
    assert_eq!(
        settings.global.client_name,
        Settings::default().global.client_name
    );
}

#[test]
fn c_flag_skips_xdg_and_does_not_create_file() {
    let dir = TempDir::new();

    let cli_path = dir.path().join("cli.toml");
    let xdg_path = dir.path().join("config.toml");
    fs::write(
        &cli_path,
        r#"
[hardware]
pad_sensitivity = 33
"#,
    )
    .unwrap();

    let loaded = load_config_with_xdg(Some(cli_path.to_str().unwrap()), &xdg_path).unwrap();

    assert_eq!(loaded.settings.hardware.pad_sensitivity, 33);
    assert!(
        !xdg_path.exists(),
        "XDG file must not be created when -c FILE is passed"
    );
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        1,
        "only the -c file exists; no XDG file written"
    );
}

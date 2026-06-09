use std::env;
use std::path::{Path, PathBuf};

use crate::settings::{PartialSettings, Settings};

const CONFIG_DIR_NAME: &str = "maschine-mikro-mk3-driver";
const CONFIG_FILE_NAME: &str = "config.toml";

const FIRST_RUN_STUB: &str = "\
# Maschine Mikro MK3 driver - user config
#
# This file holds ONLY your overrides; any key you omit uses the built-in
# default. For the full list of every available key and its default value,
# see `default_config.toml` in the project repo, or regenerate it with
# `cargo run --bin gen-default-config`.
#
# Example:
#   [hardware]
#   pad_sensitivity = 60
";

fn xdg_config_path() -> Result<PathBuf, String> {
    xdg_config_path_for(
        env::var("XDG_CONFIG_HOME").ok().as_deref(),
        env::var("HOME").ok().as_deref(),
    )
}

pub(crate) fn xdg_config_path_for(
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(xdg) = xdg_config_home.filter(|v| !v.is_empty()) {
        // XDG spec requires an absolute path; fall through to default if relative.
        if Path::new(xdg).is_absolute() {
            return Ok(PathBuf::from(xdg)
                .join(CONFIG_DIR_NAME)
                .join(CONFIG_FILE_NAME));
        }
    }

    let home = home.ok_or_else(|| "$HOME not set; cannot resolve XDG config path".to_string())?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join(CONFIG_DIR_NAME)
        .join(CONFIG_FILE_NAME))
}

fn write_atomically(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create config dir {parent:?}: {err}"))?;
    }

    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp_path = PathBuf::from(tmp);

    std::fs::write(&tmp_path, contents)
        .map_err(|err| format!("failed to write {tmp_path:?}: {err}"))?;
    std::fs::rename(&tmp_path, path).map_err(|err| {
        let _ = std::fs::remove_file(&tmp_path);
        format!("failed to rename {tmp_path:?} to {path:?}: {err}")
    })
}

fn merge_from_source(
    source: config::File<config::FileSourceFile, config::FileFormat>,
) -> Result<Settings, String> {
    let partial: PartialSettings = config::Config::builder()
        .add_source(source)
        .build()
        .map_err(|err| format!("Can't create settings: {err}"))?
        .try_deserialize()
        .map_err(|err| format!("Can't parse settings: {err}"))?;
    Ok(Settings::default().merge_overrides(partial))
}

pub fn resolve_and_load_settings(cli_config: Option<&str>) -> Result<Settings, String> {
    if let Some(file) = cli_config {
        return merge_from_source(config::File::with_name(file));
    }
    let xdg_path = xdg_config_path()?;
    load_xdg(&xdg_path)
}

pub fn load_xdg(xdg_path: &Path) -> Result<Settings, String> {
    if xdg_path.exists() {
        return merge_from_source(config::File::from(xdg_path.to_path_buf()));
    }
    if let Err(err) = write_atomically(xdg_path, FIRST_RUN_STUB) {
        eprintln!("warning: could not write config stub to {xdg_path:?}: {err}");
    }
    Ok(Settings::default())
}

/// Serialize `settings`' sparse overrides (vs defaults) to TOML and write them
/// atomically to `path`.
pub fn save_to(path: &Path, settings: &Settings) -> Result<(), String> {
    let overrides = settings.diff_from_defaults();
    let body = toml::to_string(&overrides)
        .map_err(|err| format!("failed to serialize settings: {err}"))?;
    write_atomically(path, &body)
}

/// Persist `settings`' overrides to the resolved XDG config path.
pub fn save_xdg(settings: &Settings) -> Result<(), String> {
    let path = xdg_config_path()?;
    save_to(&path, settings)
}

#[cfg(test)]
mod tests {
    use super::xdg_config_path_for;
    use super::{load_xdg, save_to};
    use crate::settings::{MidiChannel, Settings};
    use std::path::PathBuf;

    #[test]
    fn save_to_then_load_round_trips_overrides() {
        let dir = std::env::temp_dir().join("mmk3-persist-save-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("config-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut s = Settings::default();
        s.hardware.pad_sensitivity = 73;
        s.global.midi_channel = MidiChannel::try_from(4).unwrap();

        save_to(&path, &s).unwrap();
        let loaded = load_xdg(&path).unwrap();
        assert_eq!(loaded, s);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn xdg_path_uses_xdg_config_home_when_set() {
        let path = xdg_config_path_for(Some("/tmp/xdg-test"), Some("/home/ignored")).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/tmp/xdg-test/maschine-mikro-mk3-driver/config.toml")
        );
    }

    #[test]
    fn xdg_path_treats_empty_xdg_config_home_as_unset() {
        let path = xdg_config_path_for(Some(""), Some("/home/user")).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/home/user/.config/maschine-mikro-mk3-driver/config.toml")
        );
    }

    #[test]
    fn xdg_path_falls_back_to_home_dot_config_when_xdg_unset() {
        let path = xdg_config_path_for(None, Some("/home/user")).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/home/user/.config/maschine-mikro-mk3-driver/config.toml")
        );
    }

    #[test]
    fn xdg_path_errors_when_home_unset() {
        assert!(xdg_config_path_for(None, None).is_err());
    }
}

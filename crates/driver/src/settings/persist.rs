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

/// Read a TOML file into a `PartialSettings`. The format is pinned to TOML (not
/// inferred from the extension) so load and save agree on the same file for any
/// `-c` argument.
fn read_partial(path: &Path) -> Result<PartialSettings, String> {
    config::Config::builder()
        .add_source(config::File::from(path.to_path_buf()).format(config::FileFormat::Toml))
        .build()
        .map_err(|err| format!("Can't create settings: {err}"))?
        .try_deserialize()
        .map_err(|err| format!("Can't parse settings: {err}"))
}

/// Resolved startup configuration.
pub struct LoadedConfig {
    /// Live settings the driver runs with: `defaults ∘ -c seed ∘ XDG overrides`.
    pub settings: Settings,
    /// Persistence base (`defaults ∘ -c seed`). GUI edits are diffed against this
    /// and written to `persist_path`, so the read-only `-c` file shows through
    /// for keys the GUI hasn't touched.
    pub persist_base: Settings,
    /// Where GUI edits persist — always the XDG config path. A `-c` file is a
    /// read-only seed and is never written.
    pub persist_path: PathBuf,
}

/// Load the startup configuration using an explicit XDG path (testable).
///
/// `-c` (`cli_config`) is a read-only seed merged over the defaults; persisted
/// overrides live in `xdg_path` and overlay on top of the seed. A first run with
/// no `-c` writes a documented stub to `xdg_path`; with `-c` the XDG file is left
/// untouched until the first GUI persist.
pub fn load_config_with_xdg(
    cli_config: Option<&str>,
    xdg_path: &Path,
) -> Result<LoadedConfig, String> {
    let persist_base = match cli_config {
        Some(file) => Settings::default().merge_overrides(read_partial(Path::new(file))?),
        None => Settings::default(),
    };

    let settings = if xdg_path.exists() {
        persist_base
            .clone()
            .merge_overrides(read_partial(xdg_path)?)
    } else {
        if cli_config.is_none()
            && let Err(err) = write_atomically(xdg_path, FIRST_RUN_STUB)
        {
            eprintln!("warning: could not write config stub to {xdg_path:?}: {err}");
        }
        persist_base.clone()
    };

    Ok(LoadedConfig {
        settings,
        persist_base,
        persist_path: xdg_path.to_path_buf(),
    })
}

/// Load the startup configuration, resolving the XDG path from the environment.
pub fn load_config(cli_config: Option<&str>) -> Result<LoadedConfig, String> {
    let xdg_path = xdg_config_path()?;
    load_config_with_xdg(cli_config, &xdg_path)
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

/// Serialize `settings`' sparse overrides relative to `base` to TOML and write
/// them atomically to `path`. `base` is the persistence base (`defaults ∘ -c
/// seed`) so only GUI-made changes are persisted.
pub fn save_overrides(path: &Path, settings: &Settings, base: &Settings) -> Result<(), String> {
    let overrides = settings.diff_from(base);
    let body = toml::to_string(&overrides)
        .map_err(|err| format!("failed to serialize settings: {err}"))?;
    write_atomically(path, &body)
}

#[cfg(test)]
mod tests {
    use super::xdg_config_path_for;
    use super::{load_config_with_xdg, load_xdg, save_overrides};
    use crate::settings::{MidiChannel, Settings};
    use std::path::PathBuf;

    #[test]
    fn save_overrides_then_load_round_trips_overrides() {
        let dir = std::env::temp_dir().join("mmk3-persist-save-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("config-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut s = Settings::default();
        s.hardware.pad_sensitivity = 73;
        s.global.midi_channel = MidiChannel::try_from(4).unwrap();

        save_overrides(&path, &s, &Settings::default()).unwrap();
        let loaded = load_xdg(&path).unwrap();
        assert_eq!(loaded, s);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn cli_seed_is_read_only_and_xdg_overrides_layer_on_top() {
        let dir = std::env::temp_dir().join("mmk3-persist-overlay-test");
        std::fs::create_dir_all(&dir).unwrap();
        let cli = dir.join(format!("seed-{}.toml", std::process::id()));
        let xdg = dir.join(format!("xdg-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&cli);
        let _ = std::fs::remove_file(&xdg);

        // A `-c` seed sets pad sensitivity and the global channel.
        std::fs::write(
            &cli,
            "[hardware]\npad_sensitivity = 88\n[global]\nmidi_channel = 7\n",
        )
        .unwrap();

        // First load: no XDG overlay yet → live == seed, and the seed file is the
        // persistence base. The XDG file must NOT be created for a `-c` run.
        let loaded = load_config_with_xdg(Some(cli.to_str().unwrap()), &xdg).unwrap();
        assert_eq!(loaded.settings.hardware.pad_sensitivity, 88);
        assert_eq!(loaded.settings.global.midi_channel.as_u8(), 7);
        assert!(!xdg.exists(), "-c run must not create the XDG file");

        // A GUI edit changes only display_contrast; persist the diff vs the base.
        let mut edited = loaded.settings.clone();
        edited.hardware.display_contrast = 20;
        save_overrides(&xdg, &edited, &loaded.persist_base).unwrap();

        // Next load overlays the XDG diff on the seed: the edit applies and the
        // seed still shows through for untouched keys.
        let reloaded = load_config_with_xdg(Some(cli.to_str().unwrap()), &xdg).unwrap();
        assert_eq!(reloaded.settings.hardware.display_contrast, 20);
        assert_eq!(reloaded.settings.hardware.pad_sensitivity, 88);
        assert_eq!(reloaded.settings.global.midi_channel.as_u8(), 7);

        // The seed file is untouched (only display_contrast lives in XDG).
        let seed_text = std::fs::read_to_string(&cli).unwrap();
        assert!(seed_text.contains("pad_sensitivity = 88"));
        assert!(!seed_text.contains("display_contrast"));

        let _ = std::fs::remove_file(&cli);
        let _ = std::fs::remove_file(&xdg);
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

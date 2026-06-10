//! GUI-local view preferences — settings the driver does not own (they affect
//! only the client UI), persisted to a small TOML file under XDG config so they
//! survive between launches.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The persisted client-side preferences. `#[serde(default)]` lets a missing or
/// unknown field fall back to its default instead of failing the whole parse.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct GuiPrefs {
    pub show_all_labels: bool,
    pub touch_select: bool,
}

impl Default for GuiPrefs {
    fn default() -> Self {
        Self {
            show_all_labels: false,
            touch_select: true,
        }
    }
}

/// `$XDG_CONFIG_HOME/maschine-mikro-mk3-driver/gui.toml` (falling back to
/// `$HOME/.config/...`).
fn path() -> Option<PathBuf> {
    // Match the driver's resolution: an empty or relative XDG_CONFIG_HOME is not a
    // valid base (the XDG spec requires an absolute path), so fall back to
    // $HOME/.config instead of writing a cwd-relative file the next launch can't
    // find.
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("maschine-mikro-mk3-driver").join("gui.toml"))
}

impl GuiPrefs {
    /// Load preferences, falling back to defaults when the file is missing,
    /// unreadable, or malformed.
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Write the preferences (best-effort; failures are ignored). The write is
    /// atomic (temp file + rename) so an interrupted save can't leave a
    /// truncated file that mis-parses on the next launch.
    pub fn save(&self) {
        let Some(path) = path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(body) = toml::to_string(self) else {
            return;
        };
        let mut tmp = path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

impl crate::app::State {
    /// Persist the GUI-local view preferences.
    pub(crate) fn save_gui_prefs(&self) {
        GuiPrefs {
            show_all_labels: self.show_all_labels,
            touch_select: self.touch_select,
        }
        .save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let prefs = GuiPrefs {
            show_all_labels: true,
            touch_select: false,
        };
        let back: GuiPrefs = toml::from_str(&toml::to_string(&prefs).unwrap()).unwrap();
        assert!(back.show_all_labels);
        assert!(!back.touch_select);
    }

    #[test]
    fn missing_field_falls_back_to_default() {
        let back: GuiPrefs = toml::from_str("show_all_labels = true").unwrap();
        assert!(back.show_all_labels);
        assert!(back.touch_select, "absent field keeps its default (true)");
    }

    #[test]
    fn unknown_key_is_ignored() {
        let back: GuiPrefs = toml::from_str("touch_select_extra = true").unwrap();
        assert!(back.touch_select);
        assert!(!back.show_all_labels);
    }

    #[test]
    fn defaults_touch_select_on_labels_off() {
        let d = GuiPrefs::default();
        assert!(d.touch_select);
        assert!(!d.show_all_labels);
    }
}

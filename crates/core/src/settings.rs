//! User settings in one TOML file under the XDG data dir (`~/.local/share/FastClipboard`).
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::files::{read_or_backup, write_atomic};

/// Folder name under the XDG data dir.
pub const APP_DIR: &str = "FastClipboard";
pub const MIN_SIZE: (u32, u32) = (400, 300);
/// Upper bound for a hand-edited size; a huge value would fail at window creation.
pub const MAX_SIZE: u32 = 8192;

/// Range of `max_items`, for the preferences and for a hand-edited file. Beyond it the whole-file history rewrite is slow.
pub const MAX_ITEMS_RANGE: std::ops::RangeInclusive<usize> = 10..=5000;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)] // older files keep loading when a field is added
pub struct Settings {
    /// "" follows the system locale; otherwise a gettext code such as "pt_BR".
    pub language: String,
    pub width: u32,
    pub height: u32,
    /// Oldest entries beyond this are dropped.
    pub max_items: usize,
    /// Global shortcut that opens the popup: modifiers (ctrl, alt, shift, super) plus one letter or digit.
    pub hotkey: String,
    /// Tesseract languages for "Text from image", joined with `+` (e.g. "eng+por"); needs the language packs installed.
    /// "" = English plus the interface language, when its pack is installed.
    pub ocr_lang: String,
    /// "" follows the system's light or dark mode; otherwise "light" or "dark".
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: String::new(),
            width: 600,
            height: 450,
            max_items: 500,
            hotkey: "ctrl+alt+v".into(),
            ocr_lang: String::new(),
            theme: String::new(),
        }
    }
}

fn parse(text: &str) -> Result<Settings, toml::de::Error> {
    toml::from_str(text)
}

pub(crate) fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(APP_DIR)
}

fn path() -> PathBuf {
    data_dir().join("settings.toml")
}

impl Settings {
    pub fn load() -> Self {
        Self::load_from(&path())
    }

    fn load_from(path: &Path) -> Self {
        let mut s = read_or_backup(path, |text| parse(text).map_err(|e| e.to_string())).0;
        s.normalize();
        s
    }

    fn normalize(&mut self) {
        self.width = self.width.clamp(MIN_SIZE.0, MAX_SIZE);
        self.height = self.height.clamp(MIN_SIZE.1, MAX_SIZE);
        self.max_items = self.max_items.clamp(*MAX_ITEMS_RANGE.start(), *MAX_ITEMS_RANGE.end());
    }

    /// Atomic write so a crash can never leave a half-written file. Failures are reported on stderr;
    /// the app keeps working with the in-memory settings.
    pub fn save(&self) {
        if let Err(e) = self.try_save() {
            eprintln!("cannot save settings to {}: {e}", path().display());
        }
    }

    fn try_save(&self) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        write_atomic(&path(), text)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn old_files_still_load() {
        let parsed: Settings = toml::from_str("width = 700").unwrap();
        assert_eq!(
            (
                parsed.width,
                parsed.language.as_str(),
                parsed.max_items,
                parsed.hotkey.as_str(),
                parsed.ocr_lang.as_str()
            ),
            (700, "", 500, "ctrl+alt+v", "")
        );
    }

    #[test]
    fn unreadable_or_invalid_files_are_kept_as_a_backup() {
        let dir = std::env::temp_dir().join(format!("fastclipboard-load-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.toml");
        let backups = || {
            fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().contains(".bad."))
                .count()
        };
        assert_eq!(
            Settings::load_from(&file).width,
            600,
            "missing file: defaults, no backup"
        );
        assert_eq!(backups(), 0);
        fs::write(&file, [0xff, 0xfe, b'w']).unwrap(); // not UTF-8, as a Latin-1 editor would save it
        assert_eq!(Settings::load_from(&file).width, 600);
        assert!(!file.exists(), "moved aside, so the next save() cannot overwrite it");
        assert_eq!(backups(), 1);
        fs::write(&file, "width = 700").unwrap();
        assert_eq!(Settings::load_from(&file).width, 700, "a good file still loads");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_file_is_an_error_not_defaults() {
        assert!(parse("width = \"wide\"").is_err());
    }

    #[test]
    fn max_items_is_clamped_and_backups_do_not_collide() {
        for (raw, want) in [(0, 10), (100_000_000, 5000), (200, 200)] {
            let mut s: Settings = toml::from_str(&format!("max_items = {raw}")).unwrap();
            s.normalize();
            assert_eq!(s.max_items, want);
        }
        let name = crate::files::backup_path(Path::new("/x/history.toml"));
        assert!(name.to_string_lossy().starts_with("/x/history.toml.bad."));
    }

    #[test]
    fn theme_defaults_to_the_system_and_loads_from_a_file() {
        let old: Settings = toml::from_str("width = 700").unwrap();
        assert_eq!(old.theme, "", "files from before the option still load");
        let dark: Settings = toml::from_str("theme = \"dark\"").unwrap();
        assert_eq!(dark.theme, "dark");
    }

    #[test]
    fn window_size_is_clamped_both_ways() {
        let mut s: Settings = toml::from_str("width = 4000000000\nheight = 1").unwrap();
        s.normalize();
        assert_eq!((s.width, s.height), (MAX_SIZE, MIN_SIZE.1));
    }
}

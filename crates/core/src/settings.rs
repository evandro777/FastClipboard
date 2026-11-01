//! User settings in one TOML file under the XDG data dir (`~/.local/share/FastPaste`).
use std::{
    fs,
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// Folder name under the XDG data dir.
pub const APP_DIR: &str = "FastPaste";
pub const MIN_SIZE: (u32, u32) = (400, 300);
/// Upper bound for a hand-edited size; a huge value would fail at window creation.
pub const MAX_SIZE: u32 = 8192;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)] // older files keep loading when a field is added
pub struct Settings {
    /// "" follows the system locale; otherwise a gettext code such as "pt_BR".
    pub language: String,
    pub width: u32,
    pub height: u32,
    /// Oldest unpinned entries beyond this are dropped.
    pub max_items: usize,
    /// Global shortcut that opens the popup: modifiers (ctrl, alt, shift, super) plus one letter or digit.
    pub hotkey: String,
    /// Tesseract languages for "Text from image", joined with `+` (e.g. "eng+por"); needs the language packs installed.
    /// "" = English plus the interface language, when its pack is installed.
    pub ocr_lang: String,
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

/// Creates `dir` (and parents) readable only by the user, and tightens one made by an older version (0755): the
/// history holds everything that was copied, passwords included.
pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

/// Writes `data` to a file only the user can read. The mode is also set on an existing file (a leftover `.tmp`).
pub(crate) fn write_private(path: &Path, data: impl AsRef<[u8]>) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(data.as_ref())
}

impl Settings {
    pub fn load() -> Self {
        let mut s = match fs::read_to_string(path()) {
            Ok(text) => parse(&text).unwrap_or_else(|e| {
                // The file is hand-edited; the next save() must not overwrite what the user wrote.
                let bad = path().with_extension("toml.bad");
                eprintln!(
                    "settings.toml is invalid ({e}); kept as {}, using defaults",
                    bad.display()
                );
                let _ = fs::rename(path(), bad);
                Settings::default()
            }),
            Err(_) => Settings::default(),
        };
        s.normalize();
        s
    }

    fn normalize(&mut self) {
        self.width = self.width.clamp(MIN_SIZE.0, MAX_SIZE);
        self.height = self.height.clamp(MIN_SIZE.1, MAX_SIZE);
        self.max_items = self.max_items.max(1);
    }

    /// Atomic write so a crash can never leave a half-written file. Failures are reported on stderr;
    /// the app keeps working with the in-memory settings.
    pub fn save(&self) {
        if let Err(e) = self.try_save() {
            eprintln!("cannot save settings to {}: {e}", path().display());
        }
    }

    fn try_save(&self) -> std::io::Result<()> {
        let dir = data_dir();
        create_private_dir(&dir)?;
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = dir.join("settings.toml.tmp");
        write_private(&tmp, text)?;
        fs::rename(&tmp, path())
    }
}

#[cfg(test)]
mod tests {
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
    fn private_dir_and_file_are_tightened() {
        let root = std::env::temp_dir().join(format!("fastpaste-perm-{}", std::process::id()));
        let dir = root.join("data");
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let file = dir.join("x.tmp");
        fs::write(&file, "old").unwrap(); // 0644 leftover
        create_private_dir(&dir).unwrap();
        write_private(&file, "new").unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&dir), mode(&file)), (0o700, 0o600));
        assert_eq!(fs::read_to_string(&file).unwrap(), "new");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_file_is_an_error_not_defaults() {
        assert!(parse("width = \"wide\"").is_err());
    }

    #[test]
    fn window_size_is_clamped_both_ways() {
        let mut s: Settings = toml::from_str("width = 4000000000\nheight = 1").unwrap();
        s.normalize();
        assert_eq!((s.width, s.height), (MAX_SIZE, MIN_SIZE.1));
    }
}

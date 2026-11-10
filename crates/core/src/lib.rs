//! UI-independent logic: settings now, scanners/models later. Keep Slint out of this crate so a CLI can reuse it.
pub mod files;
pub mod history;
pub mod settings;
pub mod transform;

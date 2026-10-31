//! Tray icon (StatusNotifierItem): Open, Preferences, Quit. It is optional: without a tray host or a
//! D-Bus session the app just runs without it (the hotkey still works).
use ksni::{
    Icon, MenuItem, Tray,
    blocking::{Handle, TrayMethods},
    menu::StandardItem,
};

use crate::images;

/// The app icon (packaging/*.svg rendered at 64x64): a pixmap, so it shows without being installed in a theme.
const ICON: &[u8] = include_bytes!("../../../packaging/io.github.evandro777.FastPaste.png");

/// Already translated, because the tray lives on its own thread and the strings come from Slint's `Tr` global.
pub struct Labels {
    pub open: String,
    pub prefs: String,
    pub quit: String,
}

/// What the menu does; each posts work to the UI thread.
pub struct Actions {
    pub open: fn(),
    pub prefs: fn(),
    pub quit: fn(),
}

pub struct FastTray {
    labels: Labels,
    actions: Actions,
}

impl Tray for FastTray {
    fn id(&self) -> String {
        "fastpaste".into()
    }

    fn title(&self) -> String {
        "FastPaste".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let Some((width, height, rgba)) = images::decode_rgba(ICON) else {
            return Vec::new();
        };
        // StatusNotifier wants ARGB32 in network byte order.
        let data = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[3], p[0], p[1], p[2]])
            .collect();
        vec![Icon {
            width: width as i32,
            height: height as i32,
            data,
        }]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        (self.actions.open)();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: self.labels.open.clone(),
                icon_name: "edit-paste".into(),
                activate: Box::new(|t: &mut Self| (t.actions.open)()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: self.labels.prefs.clone(),
                icon_name: "preferences-system".into(),
                activate: Box::new(|t: &mut Self| (t.actions.prefs)()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: self.labels.quit.clone(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut Self| (t.actions.quit)()),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Keep the returned handle alive for as long as the tray should stay.
pub fn spawn(labels: Labels, actions: Actions) -> Option<Handle<FastTray>> {
    FastTray { labels, actions }
        .spawn()
        .inspect_err(|e| eprintln!("no tray icon ({e}); the hotkey still works"))
        .ok()
}

/// New labels after a language change.
pub fn set_labels(handle: &Handle<FastTray>, labels: Labels) {
    let _ = handle.update(|t| t.labels = labels);
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_icon_decodes() {
        let (w, h, rgba) = crate::images::decode_rgba(super::ICON).unwrap();
        assert_eq!((w, h, rgba.len()), (64, 64, 64 * 64 * 4));
    }
}

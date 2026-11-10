//! Canned data for `scripts/ui-snapshot.sh` (`FASTCLIPBOARD_FIXTURES=empty|single|many`): the popup can be rendered with zero,
//! one or many rows without reading the real history or clipboard.
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use crate::{Main, Row, Tr};

/// 0 empty, 1 single, 2 many; `None` when the variable is unset or unknown.
pub fn mode() -> Option<usize> {
    match std::env::var("FASTCLIPBOARD_FIXTURES").ok()?.as_str() {
        "empty" => Some(0),
        "single" => Some(1),
        "many" => Some(2),
        _ => None,
    }
}

/// `FASTCLIPBOARD_PAGE`: 0 the popup (default), 1 the preferences over it.
pub fn page() -> usize {
    std::env::var("FASTCLIPBOARD_PAGE")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(0)
}

/// `FASTCLIPBOARD_OCR=noperm|notesseract|missing`: the OCR state to draw (default ready). tesseract is never run.
fn ocr_status() -> crate::ocr::Status {
    use crate::ocr::Status;
    match std::env::var("FASTCLIPBOARD_OCR").as_deref() {
        Ok("noperm") => Status::NoPermission,
        Ok("notesseract") => Status::NoTesseract,
        Ok("missing") => Status::MissingLang(vec!["por".into(), "spa".into()]),
        _ => Status::Ready,
    }
}

/// Preferences over the popup; the three modes show the defaults, Portuguese selected with the clear-history question,
/// and the shortcut-taken error. `FASTCLIPBOARD_ABOUT=1` puts the About card on top.
fn prefs(ui: &Main, mode: usize) {
    ui.set_settings_open(true);
    ui.set_hotkey("Ctrl+Alt+V".into());
    ui.set_max_items(500);
    ui.set_language(if mode == 1 { 2 } else { 0 });
    ui.set_error(if mode == 2 { 2 } else { 0 });
    ui.set_autostart(mode == 1);
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    crate::app::fill_ocr(ui, &ocr_status());
    ui.set_confirm_clear(mode == 1);
    ui.set_about_open(std::env::var("FASTCLIPBOARD_ABOUT").is_ok_and(|v| v == "1"));
}

const TEXTS: [&str; 12] = [
    "https://github.com/slint-ui/slint",
    "cargo build --profile fast",
    "Olá, mundo! Áéíóú çãõ",
    "SELECT id, name FROM users WHERE active = 1 ORDER BY name LIMIT 50; -- a long query that should be cut with an ellipsis at the right edge of the window",
    "jane.doe@example.com",
    "192.168.0.42",
    "fn main() { println!(\"hello\"); }",
    "TODO: reply to the meeting invite",
    "/home/user/projects/app/src/main.rs",
    "#1e90ff",
    "The quick brown fox jumps over the lazy dog",
    "42",
];

/// A 192x96 gradient standing in for a stored thumbnail.
fn fake_thumbnail() -> Image {
    let (w, h) = (192u32, 96u32);
    let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    for (i, p) in pixels.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        *p = Rgba8Pixel {
            r: (x * 255 / w) as u8,
            g: (y * 255 / h) as u8,
            b: 160,
            a: 255,
        };
    }
    Image::from_rgba8(pixels)
}

/// Use `[0, 1, 12][mode]` as the row count, so one fixture covers all three states. Rows 1 and 7 of the long list are
/// images: one with a thumbnail, one without (too big to decode).
pub fn apply(ui: &Main, mode: usize) {
    let count = [0, 1, 12][mode];
    let tr = ui.global::<Tr>();
    let mut top = 0.0;
    let rows: Vec<Row> = TEXTS
        .iter()
        .take(count)
        .enumerate()
        .map(|(i, t)| match (mode, i) {
            (2, 1) | (2, 7) => Row {
                text: tr.invoke_image(1920, 1080, 842),
                has_thumb: i == 1,
                is_image: true,
                thumb: if i == 1 { fake_thumbnail() } else { Image::default() },
                ..Default::default()
            },
            _ => Row {
                text: (*t).into(),
                ..Default::default()
            },
        })
        .map(|mut row| {
            row.top = top;
            row.height = if row.has_thumb { 74.0 } else { 38.0 };
            top += row.height;
            row
        })
        .collect();
    ui.set_rows(ModelRc::new(VecModel::from(rows)));
    // Row 1 is an image, so the "many" snapshot shows the image menu (OCR entries); text rows list no OCR.
    ui.set_current(if mode == 2 { 1 } else { 0 });
    // The "Show all" button is part of the "many" snapshot.
    ui.set_hidden(if mode == 2 { 30 } else { 0 });
    // The Shift+Enter menu is part of the "many" snapshot of the popup.
    ui.set_menu_open(mode == 2 && page() == 0);
    ui.set_ocr_ready(ocr_status() == crate::ocr::Status::Ready);
    if page() == 1 {
        prefs(ui, mode);
    }
}

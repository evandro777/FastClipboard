//! Canned data for `scripts/ui-snapshot.sh` (`FASTPASTE_FIXTURES=empty|single|many`): the popup can be rendered with zero,
//! one or many rows without reading the real history or clipboard.
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use crate::{Main, Preferences, Row, Tr};

/// 0 empty, 1 single, 2 many; `None` when the variable is unset or unknown.
pub fn mode() -> Option<usize> {
    match std::env::var("FASTPASTE_FIXTURES").ok()?.as_str() {
        "empty" => Some(0),
        "single" => Some(1),
        "many" => Some(2),
        _ => None,
    }
}

/// `FASTPASTE_PAGE`: 0 the popup (default), 1 the preferences window.
pub fn page() -> usize {
    std::env::var("FASTPASTE_PAGE")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(0)
}

/// `FASTPASTE_OCR=noperm|notesseract|missing`: the OCR state to draw (default ready). tesseract is never run.
fn ocr_status() -> crate::ocr::Status {
    use crate::ocr::Status;
    match std::env::var("FASTPASTE_OCR").as_deref() {
        Ok("noperm") => Status::NoPermission,
        Ok("notesseract") => Status::NoTesseract,
        Ok("missing") => Status::MissingLang(vec!["por".into(), "spa".into()]),
        _ => Status::Ready,
    }
}

/// The preferences window; the three modes show the defaults, Portuguese selected, and an error message.
pub fn prefs(mode: usize) -> Preferences {
    let win = Preferences::new().unwrap();
    win.set_hotkey("ctrl+alt+v".into());
    win.set_max_items(if mode == 2 { "99999" } else { "500" }.into());
    win.set_language(if mode == 1 { 2 } else { 0 });
    win.set_error(if mode == 2 { 3 } else { 0 });
    win.set_autostart(mode == 1);
    win.set_version(env!("CARGO_PKG_VERSION").into());
    crate::app::fill_ocr(&win, &ocr_status());
    // The overlays are part of the snapshots: the clear-history warning (single) and the About screen (many).
    win.set_confirm_clear(mode == 1);
    win.set_about_open(mode == 2);
    win.show().unwrap();
    win
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
    // The Shift+Enter menu is part of the "many" snapshot.
    ui.set_menu_open(mode == 2);
    ui.set_ocr_ready(ocr_status() == crate::ocr::Status::Ready);
}

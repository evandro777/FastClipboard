use fastpaste_core::settings::Settings;
use slint::ComponentHandle;

slint::include_modules!();

mod app;
mod autostart;
mod fixtures;
mod images;
mod instance;
mod ocr;
mod tray;
mod x11;

fn main() {
    if std::env::args().skip(1).any(|a| a == "-V" || a == "--version") {
        return println!("fastpaste {}", env!("CARGO_PKG_VERSION"));
    }
    // Fixtures never talk to a real running instance (scripts/ui-snapshot.sh).
    if fixtures::mode().is_none() && instance::send(instance::command()) {
        return;
    }
    // Forced so the 100 MB GL renderer is never pulled in by accident.
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()
        .expect("cannot start the Slint software renderer");

    // Fixtures must not read (or, for an invalid file, rename) the real settings.
    let settings = if fixtures::mode().is_some() {
        Settings::default()
    } else {
        Settings::load()
    };
    let ui = Main::new().unwrap();
    apply_language(&settings);
    ui.window()
        .set_size(slint::LogicalSize::new(settings.width as f32, settings.height as f32));

    // With fixtures (scripts/ui-snapshot.sh) the real history and clipboard are never touched.
    if let Some(mode) = fixtures::mode() {
        let _prefs; // alive for the event loop
        if fixtures::page() == 1 {
            _prefs = fixtures::prefs(mode);
        } else {
            fixtures::apply(&ui, mode);
            ui.show().unwrap();
        }
        return slint::run_event_loop_until_quit().unwrap();
    }
    app::run(ui, settings);
}

/// Switches the UI language in place; one without a translation falls back to the English source strings.
fn apply_language(settings: &Settings) {
    if slint::select_bundled_translation(&language(settings)).is_err() {
        let _ = slint::select_bundled_translation("");
    }
}

/// `settings.language`, or the system locale (`LC_ALL`/`LC_MESSAGES`/`LANGUAGE`/`LANG`, minus `.UTF-8`).
fn language(settings: &Settings) -> String {
    if !settings.language.is_empty() {
        return settings.language.clone();
    }
    ["LC_ALL", "LC_MESSAGES", "LANGUAGE", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .map(|v| v.split(['.', ':']).next().unwrap_or("").to_string())
        .unwrap_or_default()
}

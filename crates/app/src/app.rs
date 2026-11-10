//! The popup: list state, window focus lifecycle and paste. The focus and paste sequencing follows FastMoji, which was
//! tested on Cinnamon (Muffin); see its AGENTS.md "Hard-won gotchas" before changing the timings.
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use fastpaste_core::{
    history::History,
    settings::{MAX_ITEMS_RANGE, MIN_SIZE, Settings},
    transform::Transform,
};
use slint::{
    ComponentHandle, ModelRc, PhysicalPosition, Timer, VecModel,
    winit_030::{
        EventResult, WinitWindowAccessor,
        winit::{
            self,
            raw_window_handle::{HasWindowHandle, RawWindowHandle},
        },
    },
};

use crate::{
    Main, Row, Tr, autostart, images, instance, ocr,
    tray::{self, Actions, FastTray, Labels},
    x11,
};

/// Language codes behind the preferences' language list (`languages` in main.slint, same order); "" = the system's.
const LANGUAGES: [&str; 3] = ["", "en", "pt_BR"];

/// `settings.theme` for each entry of the Appearance list (`appearances` in main.slint, same order): system, light, dark.
const THEMES: [&str; 3] = ["", "light", "dark"];

/// The Appearance list index of a `settings.theme` value; anything unknown follows the system.
pub fn theme_index(theme: &str) -> i32 {
    THEMES.iter().position(|t| *t == theme).unwrap_or(0) as i32
}

/// Row heights in logical pixels (rows are laid out by `Row.height` in main.slint; image rows hold a 64 px thumbnail).
const TEXT_ROW: f32 = 38.0;
const IMAGE_ROW: f32 = 74.0;
/// Rows per page; the next page loads when the scroll or the arrows reach the end. Searching lists every match.
const PAGE: usize = 20;
/// Grace period after showing during which the WM may bounce focus before we trust a focus-out.
const FOCUS_GRACE: Duration = Duration::from_millis(300);
const PASTE_POLL: Duration = Duration::from_millis(25);
const PASTE_MAX_POLLS: u32 = 40;

/// What a pick puts on the clipboard.
enum Payload {
    Text(String, Option<String>),
    Image(Vec<u8>),
}

/// What the popup lists: `rows` are indexes into the history, in display order.
#[derive(Default)]
struct View {
    query: String,
    /// Rows listed beyond the first `PAGE` (grows by a page each time the end is reached).
    more: usize,
    rows: Vec<usize>,
    /// `History::rev` when `rows` was built; a copy since then shifted the indexes.
    rev: u64,
}

/// The history index behind list row `row`, or `None` when the list is out of date (or the row does not exist).
fn resolve(view: &View, history: &History, row: i32) -> Option<usize> {
    let idx = *view.rows.get(usize::try_from(row).ok()?)?;
    (view.rev == history.rev() && idx < history.items().len()).then_some(idx)
}

struct App {
    ui: slint::Weak<Main>,
    history: Arc<Mutex<History>>,
    view: RefCell<View>,
    x: x11::Handle,
    /// The remembered popup size and everything the preferences change.
    settings: RefCell<Settings>,
    /// Read by the copy threads, so a new cap applies to the next copy.
    max_items: Arc<AtomicUsize>,
    /// Kept to retranslate the menu when the language changes.
    tray: RefCell<Option<ksni::blocking::Handle<FastTray>>>,
    /// Event time of the last hotkey press, written by the hotkey thread.
    hotkey_time: Arc<AtomicU32>,
    /// The window that had focus before the popup opened; it receives the paste.
    previous: Cell<Option<u32>>,
    shown_at: Cell<Option<Instant>>,
    /// When the last focus-out closed the popup. The hotkey itself takes focus for a moment (X grab), and its
    /// toggle arrives after that: without this it would reopen the popup it was meant to close.
    closed_by_focus_at: Cell<Option<Instant>>,
    focused: Cell<bool>,
    focus_attempts: Cell<u32>,
    focus_timer: Timer,
    paste_attempts: Cell<u32>,
    paste_timer: Timer,
    /// Last OCR check. Starts as Ready so nothing is greyed out before the first check ends.
    ocr_status: RefCell<ocr::Status>,
    /// A check is running; fast hotkey presses must not stack them.
    ocr_checking: Arc<AtomicBool>,
    /// Thumbnails of the rows last listed, so "load more" does not decode the earlier ones again. Rebuilt on every
    /// refresh from the rows shown, so it never holds an image that left the list.
    thumbs: RefCell<HashMap<String, slint::Image>>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

/// Runs `f` on the app from any thread (it goes through the Slint event loop).
fn post(f: impl FnOnce(&App) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        APP.with(|app| {
            if let Some(app) = app.borrow().as_ref() {
                f(app);
            }
        })
    });
}

/// A timer callback that runs `f` on the app (timers outlive any borrow, so they look it up each tick).
fn tick(f: fn(&App)) -> impl FnMut() + 'static {
    move || {
        APP.with(|app| {
            if let Some(app) = app.borrow().as_ref() {
                f(app);
            }
        })
    }
}

fn debug(message: impl FnOnce() -> String) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("FASTPASTE_DEBUG").is_some()) {
        eprintln!("{}", message());
    }
}

pub fn run(ui: Main, settings: Settings) {
    let history = Arc::new(Mutex::new(History::load()));
    let hotkey_time = Arc::new(AtomicU32::new(0));
    let max_items = Arc::new(AtomicUsize::new(settings.max_items));
    let on_copy = {
        let (history, max) = (history.clone(), max_items.clone());
        move |text: String, html: Option<String>| {
            debug(|| format!("copy recorded: {} bytes, html {}", text.len(), html.is_some()));
            let mut h = history.lock().unwrap();
            if h.push(text, html, max.load(Ordering::SeqCst)) {
                h.save();
            }
        }
    };
    let on_image = {
        let (history, max) = (history.clone(), max_items.clone());
        // One worker, not a thread per copy: the X11 thread (hotkey, serving the clipboard) must not wait for a decode,
        // and a burst of big images is processed one at a time, in order.
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            for data in rx {
                // Validated and thumbnailed before taking the lock: decoding takes a moment.
                let png = data;
                let Some(size) = images::size(&png) else {
                    eprintln!("image skipped: not a valid PNG");
                    continue;
                };
                let thumb = match images::thumbnail(&png) {
                    images::Thumb::Made(t) => Some(t),
                    images::Thumb::Skipped => None,
                    images::Thumb::Broken => {
                        eprintln!("image skipped: the PNG data is damaged");
                        continue;
                    }
                };
                debug(|| {
                    format!(
                        "image recorded: {} bytes, {}x{}, thumbnail {}",
                        png.len(),
                        size.0,
                        size.1,
                        thumb.is_some()
                    )
                });
                let mut h = history.lock().unwrap();
                if h.push_image(&png, thumb.as_deref(), size, max.load(Ordering::SeqCst)) {
                    h.save();
                } else {
                    // push_image reports a write error itself; the other reason is the size cap.
                    eprintln!(
                        "image skipped: {} bytes as PNG, the limit is {} (or it could not be stored)",
                        png.len(),
                        fastpaste_core::history::MAX_IMAGE_BYTES
                    );
                }
            }
        });
        move |data: Vec<u8>| {
            let _ = tx.send(data);
        }
    };
    let on_hotkey = {
        let time = hotkey_time.clone();
        move |t| {
            time.store(t, Ordering::SeqCst);
            post(App::hotkey_pressed);
        }
    };
    let hotkey_time_cmd = hotkey_time.clone();
    // Lost the start-up race to another instance: `claim` handed it our command, so leave.
    if instance::claim(crate::command(), move |command| {
        // No hotkey event behind a socket command, so no timestamp (0 = CurrentTime).
        hotkey_time_cmd.store(0, Ordering::SeqCst);
        match command.as_str() {
            "toggle" => post(App::toggle),
            "show" => post(|app| app.open(0)),
            "prefs" => post(App::open_prefs),
            _ => {}
        }
    }) {
        return;
    }
    let x = x11::start(on_copy, on_image, on_hotkey).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    // A shortcut that is taken (or invalid) must not end the app: the tray and the preferences are how it gets fixed.
    let hotkey_error = x.set_hotkey(&settings.hotkey).err().map(|e| {
        eprintln!("{e}");
        // Same codes as `record_key`: 1 not valid, 2 taken.
        if x11::parse_hotkey(&settings.hotkey).is_none() {
            1
        } else {
            2
        }
    });

    let app = Rc::new(App {
        ui: ui.as_weak(),
        history,
        view: RefCell::new(View::default()),
        x,
        settings: RefCell::new(settings),
        max_items,
        tray: RefCell::new(None),
        hotkey_time,
        previous: Cell::new(None),
        shown_at: Cell::new(None),
        closed_by_focus_at: Cell::new(None),
        focused: Cell::new(false),
        focus_attempts: Cell::new(0),
        focus_timer: Timer::default(),
        paste_attempts: Cell::new(0),
        paste_timer: Timer::default(),
        ocr_status: RefCell::new(ocr::Status::Ready),
        ocr_checking: Arc::new(AtomicBool::new(false)),
        thumbs: RefCell::default(),
    });
    APP.with(|a| *a.borrow_mut() = Some(app.clone()));
    app.check_ocr();

    ui.on_query_changed({
        let app = app.clone();
        move |q| {
            app.view.borrow_mut().query = q.to_string();
            app.refresh();
        }
    });
    ui.on_show_more({
        let app = app.clone();
        move || {
            app.view.borrow_mut().more += PAGE;
            // The selection stays where it is: the next page is appended below it.
            app.refresh();
        }
    });
    ui.on_activate({
        let app = app.clone();
        move |row, plain| app.pick(row, plain)
    });
    ui.on_remove({
        let app = app.clone();
        move |row| app.remove(row)
    });
    ui.on_choose({
        let app = app.clone();
        move |row, option| app.choose(row, option)
    });
    ui.on_dismiss({
        let app = app.clone();
        move || app.hide()
    });
    let weak_app = Rc::downgrade(&app);
    ui.window().on_winit_window_event(move |_, event| {
        if let winit::event::WindowEvent::Focused(focused) = event
            && let Some(app) = weak_app.upgrade()
        {
            debug(|| format!("focused={focused}"));
            if *focused {
                app.focused.set(true);
            } else {
                app.on_focus_lost();
            }
        }
        EventResult::Propagate
    });

    ui.on_open_prefs(|| post(App::open_prefs));
    ui.on_set_theme({
        let app = app.clone();
        move |i| app.set_theme(i)
    });
    ui.on_set_language({
        let app = app.clone();
        move |i| app.set_language(i)
    });
    ui.on_set_max_items({
        let app = app.clone();
        move |n| app.set_max_items(n)
    });
    ui.on_record_key({
        let app = app.clone();
        move |text, ctrl, alt, shift, meta| app.record_key(&text, [ctrl, alt, shift, meta])
    });
    let weak = ui.as_weak();
    ui.on_set_autostart(move |on| {
        let weak = weak.clone();
        // A thread: in Flatpak the portal blocks while the desktop asks the user.
        std::thread::spawn(move || {
            if let Err(e) = autostart::set(on) {
                eprintln!("cannot change the autostart entry: {e}");
                // Put the switch back to what is really set.
                let _ = weak.upgrade_in_event_loop(|w| w.set_autostart(autostart::is_enabled()));
            }
        });
    });
    ui.on_clear_history(|| post(App::clear_history));
    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    ui.on_open_url(|url| {
        // Reaped on a thread, or every click would leave a zombie in this long-lived process.
        if let Ok(mut child) = std::process::Command::new("xdg-open").arg(url.as_str()).spawn() {
            std::thread::spawn(move || _ = child.wait());
        }
    });
    ui.on_held_text(|ctrl, alt, shift, meta| held_text([ctrl, alt, shift, meta]).into());
    *app.tray.borrow_mut() = tray::spawn(
        tray_labels(&ui),
        Actions {
            open: || post(|app| app.open(0)),
            prefs: || post(App::open_prefs),
            quit: || {
                let _ = slint::invoke_from_event_loop(|| {
                    let _ = slint::quit_event_loop();
                });
            },
        },
    );

    if let Some(code) = hotkey_error {
        // After `open_prefs`, which clears the error.
        post(move |app| {
            app.open_prefs();
            app.ui().set_error(code);
        });
    }

    // The window starts hidden; the hotkey shows it, so closing it must not end the app.
    slint::run_event_loop_until_quit().unwrap();
}

/// Preferences, About or the clear-history question is open over the list.
fn ui_has_dialog(ui: &Main) -> bool {
    ui.get_settings_open() || ui.get_about_open() || ui.get_confirm_clear()
}

pub fn fill_ocr(ui: &Main, status: &ocr::Status) {
    ui.set_ocr_state(status.code());
    ui.set_ocr_missing(status.missing().into());
    ui.set_ocr_command(status.command().into());
}

fn tray_labels(ui: &Main) -> Labels {
    let tr = ui.global::<Tr>();
    Labels {
        open: tr.invoke_open().into(),
        prefs: tr.invoke_preferences().into(),
        quit: tr.invoke_quit().into(),
    }
}

impl App {
    fn ui(&self) -> Main {
        self.ui.upgrade().expect("window is gone")
    }

    /// The global shortcut only opens the popup or brings it back to the front; it never closes it (Esc, a click
    /// elsewhere or a paste do). `--toggle` still toggles.
    fn hotkey_pressed(&self) {
        let time = self.hotkey_time.load(Ordering::SeqCst);
        if self.ui().window().is_visible() && !self.ui().get_recording_shortcut() {
            self.activate(time);
            self.ui().invoke_focus_search();
        } else {
            self.toggle();
        }
    }

    fn toggle(&self) {
        let ui = self.ui();
        // The grab swallows the current shortcut while the preferences record a new one: pressing it again means "keep it".
        if ui.get_recording_shortcut() {
            ui.set_recording_shortcut(false);
        } else if ui.window().is_visible() {
            self.hide();
        } else if self.closed_by_focus_at.get().is_some_and(|t| t.elapsed() < FOCUS_GRACE) {
            debug(|| "toggle ignored: the focus-out of this same keypress already closed the popup".into());
        } else {
            self.open(self.hotkey_time.load(Ordering::SeqCst));
        }
    }

    fn open(&self, time: u32) {
        debug(|| format!("open time={time}"));
        // A paste still waiting for the previous window would hit whatever is active now.
        self.paste_timer.stop();
        let ui = self.ui();
        if !ui.window().is_visible() {
            self.previous.set(self.x.active_window());
        }
        *self.view.borrow_mut() = View::default();
        ui.set_query("".into());
        ui.set_current(0);
        ui.set_menu_open(false);
        // A dialog left open when the popup was hidden (Esc on the list never closes one) must not come back.
        ui.set_settings_open(false);
        ui.set_about_open(false);
        ui.set_confirm_clear(false);
        ui.set_recording_shortcut(false);
        self.refresh();
        // Only while broken: a working setup is not re-checked on every open.
        if *self.ocr_status.borrow() != ocr::Status::Ready {
            self.check_ocr();
        }
        let (width, height) = {
            let s = self.settings.borrow();
            (s.width as f32, s.height as f32)
        };
        ui.window().set_size(slint::LogicalSize::new(width, height));
        // The WM must hear the position before the window is mapped, or it shows it at its own spot first and the
        // popup visibly jumps: a position hint on the existing window, and set_position for the very first show.
        let position = self.target_position(width, height);
        if let Some((x, y)) = position {
            if let Some(id) = self.window_id() {
                self.x.set_position_hint(id, x, y);
            }
            ui.window().set_position(PhysicalPosition::new(x, y));
        }
        self.shown_at.set(Some(Instant::now()));
        let _ = ui.show();
        self.activate(time);
        ui.invoke_focus_search();
        self.keep_requesting_focus();
    }

    fn hide(&self) {
        debug(|| "hide".into());
        self.focus_timer.stop();
        let ui = self.ui();
        if ui.window().is_visible() {
            self.remember_size();
        }
        let _ = ui.hide();
    }

    /// Saves the popup size when the user resized it.
    fn remember_size(&self) {
        let ui = self.ui();
        let window = ui.window();
        let size = window.size().to_logical(window.scale_factor());
        let (w, h) = (size.width.round() as u32, size.height.round() as u32);
        let mut s = self.settings.borrow_mut();
        if w >= MIN_SIZE.0 && h >= MIN_SIZE.1 && (w, h) != (s.width, s.height) {
            s.width = w;
            s.height = h;
            s.save();
        }
    }

    /// Where the popup goes: centered on the mouse pointer, kept inside the monitor the pointer is on.
    /// `None` when the pointer cannot be read (the WM then picks a spot).
    fn target_position(&self, width: f32, height: f32) -> Option<(i32, i32)> {
        let ui = self.ui();
        let window = ui.window();
        let scale = window.scale_factor();
        let (w, h) = ((width * scale) as i32, (height * scale) as i32);
        let (px, py) = self.x.pointer()?;
        // The monitors are known only once the window exists; before that the whole screen is the bound.
        let monitors = window
            .with_winit_window(|win| {
                win.available_monitors()
                    .map(|m| {
                        (
                            m.position().x,
                            m.position().y,
                            m.size().width as i32,
                            m.size().height as i32,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let (mx, my, mw, mh) = monitors
            .into_iter()
            .find(|&(x, y, w, h)| (x..x + w).contains(&px) && (y..y + h).contains(&py))
            .or_else(|| self.x.screen_size().map(|(w, h)| (0, 0, w, h)))
            .unwrap_or((0, 0, i32::MAX, i32::MAX));
        let x = (px - w / 2).clamp(mx, mx + (mw - w).max(0));
        let y = (py - h / 2).clamp(my, my + (mh - h).max(0));
        debug(|| format!("place at {x},{y} (pointer {px},{py}, monitor {mx},{my} {mw}x{mh}, popup {w}x{h})"));
        Some((x, y))
    }

    /// Preferences: Clear history (after its confirmation).
    fn clear_history(&self) {
        {
            let mut h = self.history.lock().unwrap();
            h.clear();
            h.save();
        }
        if self.ui().window().is_visible() {
            self.refresh();
        }
    }

    /// The X id of the popup's window; `None` until it has been shown once.
    fn window_id(&self) -> Option<u32> {
        self.ui()
            .window()
            .with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
                RawWindowHandle::Xlib(h) => Some(h.window as u32),
                RawWindowHandle::Xcb(h) => Some(h.window.get()),
                _ => None,
            })
            .flatten()
    }

    /// Hand focus to the popup using the hotkey's timestamp.
    fn activate(&self, time: u32) {
        if let Some(id) = self.window_id()
            && let Err(e) = self.x.activate(id, time)
        {
            eprintln!("could not activate window: {e}");
        }
    }

    /// The WM may ignore the first request; keep asking until the popup actually has focus.
    fn keep_requesting_focus(&self) {
        self.focused.set(false);
        self.focus_attempts.set(0);
        self.focus_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            tick(App::retry_focus),
        );
    }

    fn retry_focus(&self) {
        let attempts = self.focus_attempts.get() + 1;
        self.focus_attempts.set(attempts);
        if self.focused.get() || attempts > 6 || !self.ui().window().is_visible() {
            self.focus_timer.stop();
            return;
        }
        debug(|| format!("retry focus #{attempts}"));
        self.activate(self.hotkey_time.load(Ordering::SeqCst));
        self.ui().invoke_focus_search();
    }

    /// Whether the pointer is over the popup including its decorations, with a margin because the pointer can run
    /// ahead of the border while it is being dragged.
    fn pointer_on_frame(&self) -> bool {
        const MARGIN: i32 = 24;
        let Some((px, py)) = self.x.pointer() else {
            return false;
        };
        let rect = self.ui().window().with_winit_window(|w| {
            let pos = w.outer_position().ok()?;
            let size = w.outer_size();
            Some((pos.x, pos.y, size.width as i32, size.height as i32))
        });
        rect.flatten().is_some_and(|(x, y, w, h)| {
            (x - MARGIN..x + w + MARGIN).contains(&px) && (y - MARGIN..y + h + MARGIN).contains(&py)
        })
    }

    fn on_focus_lost(&self) {
        self.focused.set(false);
        if self.shown_at.get().is_some_and(|t| t.elapsed() < FOCUS_GRACE) {
            // The WM sometimes bounces focus right after mapping; ask again instead of hiding.
            self.activate(self.hotkey_time.load(Ordering::SeqCst));
        } else if ui_has_dialog(&self.ui()) {
            // Preferences and About stay while the user works elsewhere (copying the OCR command into a terminal).
            debug(|| "focus-out ignored: a dialog is open".into());
        } else if self.window_id().is_some_and(|id| self.x.input_focus_inside(id)) {
            // Pressing or releasing a grabbed key sends the window a focus-out (mode Grab/Ungrab) although the X
            // input focus never moved; only the server can tell that from a real focus change.
            debug(|| "focus-out without a focus change (key grab): ignored".into());
        } else if self.pointer_on_frame() {
            // A click on the title bar or a border (to move or resize) takes the focus to the WM's frame.
            debug(|| "focus-out ignored: the pointer is on the window frame".into());
        } else if self.ui().window().is_visible() {
            self.closed_by_focus_at.set(Some(Instant::now()));
            self.hide();
        }
    }

    /// Rebuilds the list from the history for the current query. The history lock and the `View` borrow are both
    /// released before the model is set, because setting it makes Slint re-read the model.
    fn refresh(&self) {
        let (entries, hidden) = {
            let h = self.history.lock().unwrap();
            let mut v = self.view.borrow_mut();
            v.rows = h.search(&v.query);
            v.rev = h.rev();
            let total = v.rows.len();
            if v.query.trim().is_empty() {
                let limit = PAGE + v.more;
                v.rows.truncate(limit);
            }
            let entries: Vec<_> = v
                .rows
                .iter()
                .map(|&i| (one_line(&h.items()[i].text), h.items()[i].image.clone()))
                .collect();
            (entries, total - v.rows.len())
        };
        let ui = self.ui();
        let tr = ui.global::<Tr>();
        let mut top = 0.0;
        let mut old_thumbs = self.thumbs.take();
        let mut thumbs = HashMap::new();
        let rows: Vec<Row> = entries
            .into_iter()
            .map(|(text, image)| {
                let mut row = match image {
                    // The label and the tiny stored thumbnail: a full screenshot is never decoded for the list.
                    Some(im) => {
                        let thumb = if im.thumb {
                            old_thumbs.remove(&im.id).or_else(|| images::load_thumbnail(&im.id))
                        } else {
                            None
                        };
                        if let Some(t) = &thumb {
                            thumbs.insert(im.id.clone(), t.clone());
                        }
                        Row {
                            text: tr.invoke_image(im.width as i32, im.height as i32, im.bytes.div_ceil(1024) as i32),
                            has_thumb: thumb.is_some(),
                            thumb: thumb.unwrap_or_default(),
                            is_image: true,
                            ..Default::default()
                        }
                    }
                    None => Row {
                        text: text.into(),
                        ..Default::default()
                    },
                };
                row.top = top;
                row.height = if row.has_thumb { IMAGE_ROW } else { TEXT_ROW };
                top += row.height;
                row
            })
            .collect();
        *self.thumbs.borrow_mut() = thumbs;
        ui.set_hidden(hidden as i32);
        ui.set_rows(ModelRc::new(VecModel::from(rows)));
    }

    /// `resolve` for the current list. A stale list is rebuilt (after the caller lets go of the history lock) and the
    /// pick is dropped: a copy arrived while the popup was open, and acting on the old row numbers would hit another item.
    fn resolve_row(&self, history: &History, row: i32) -> Option<usize> {
        let idx = resolve(&self.view.borrow(), history, row);
        if idx.is_none() {
            post(App::refresh);
        }
        idx
    }

    /// Puts the row on the clipboard, closes the popup and pastes into the window that had focus before it.
    /// `plain` drops the HTML; an image has no plain version and is pasted as it is.
    fn pick(&self, row: i32, plain: bool) {
        let payload = {
            let mut h = self.history.lock().unwrap();
            let Some(idx) = self.resolve_row(&h, row) else {
                return;
            };
            let item = h.items()[idx].clone();
            let payload = match &item.image {
                Some(image) => match h.read_png(&image.id) {
                    Some(png) => Payload::Image(png),
                    None => {
                        eprintln!("the file of image {} is missing", image.id);
                        return;
                    }
                },
                None => Payload::Text(item.text, if plain { None } else { item.html }),
            };
            if !plain || matches!(payload, Payload::Image(_)) {
                // Moves the pick to the top, like a fresh copy.
                h.move_to_top(idx);
                h.save();
            }
            payload
        };
        self.paste_payload(payload);
    }

    /// Shift+Enter menu: 0 original, 1 plain text, then the transforms. Transformed text is not added to the history,
    /// like a plain paste.
    /// On an image row only "Original" pastes the picture; the other options paste the text OCR finds in it, with the
    /// transform applied.
    fn choose(&self, row: i32, option: i32) {
        let transform = Transform::from_index(option - 2);
        let text = {
            let h = self.history.lock().unwrap();
            let Some(idx) = self.resolve_row(&h, row) else {
                return;
            };
            let item = &h.items()[idx];
            if item.image.is_some() {
                drop(h);
                if option == 0 {
                    return self.pick(row, false);
                }
                if *self.ocr_status.borrow() != ocr::Status::Ready {
                    // The preferences explain what is missing; the popup stays open while they are, so the user can
                    // copy the command into a terminal.
                    return self.open_prefs();
                }
                return self.ocr(row, transform);
            }
            match transform {
                Some(t) => t.apply(&item.text),
                None => {
                    drop(h);
                    return self.pick(row, option == 1);
                }
            }
        };
        self.paste_payload(Payload::Text(text, None));
    }

    /// Checks tesseract and the language packs in a thread and updates the menu and the preferences when it ends.
    fn check_ocr(&self) {
        if self.ocr_checking.swap(true, Ordering::SeqCst) {
            return;
        }
        let (lang, ui_lang) = {
            let s = self.settings.borrow();
            (s.ocr_lang.clone(), crate::language(&s))
        };
        let checking = self.ocr_checking.clone();
        std::thread::spawn(move || {
            let status = ocr::status(&lang, &ui_lang);
            checking.store(false, Ordering::SeqCst);
            post(move |app| app.set_ocr_status(status));
        });
    }

    fn set_ocr_status(&self, status: ocr::Status) {
        let ui = self.ui();
        ui.set_ocr_ready(status == ocr::Status::Ready);
        fill_ocr(&ui, &status);
        *self.ocr_status.borrow_mut() = status;
    }

    /// Pastes the text tesseract finds in the picture (not added to the history), with `transform` applied.
    fn ocr(&self, row: i32, transform: Option<Transform>) {
        let png = {
            let h = self.history.lock().unwrap();
            let Some(idx) = self.resolve_row(&h, row) else {
                return;
            };
            h.items()[idx].image.as_ref().and_then(|im| h.read_png(&im.id))
        };
        let Some(png) = png else {
            return; // a text row, or the file is gone
        };
        let (lang, ui_lang) = {
            let s = self.settings.borrow();
            (s.ocr_lang.clone(), crate::language(&s))
        };
        // Closed right away: the popup would otherwise sit there for the seconds tesseract takes.
        self.hide();
        std::thread::spawn(move || {
            let lang = if lang.is_empty() {
                ocr::auto_lang(&ui_lang)
            } else {
                lang
            };
            match ocr::extract(&png, &lang) {
                Ok(text) if !text.is_empty() => {
                    let text = transform.map_or(text.clone(), |t| t.apply(&text));
                    post(move |app| app.paste_payload(Payload::Text(text, None)))
                }
                Ok(_) => eprintln!("ocr: no text found in the image"),
                Err(e) => eprintln!("ocr: {e}"),
            }
        });
    }

    /// Opens the popup if needed and shows Preferences over it (burger button, tray, `--prefs`, a missing OCR setup).
    pub fn open_prefs(&self) {
        let ui = self.ui();
        if !ui.window().is_visible() {
            self.open(0);
        }
        {
            let s = self.settings.borrow();
            ui.set_hotkey(shown_hotkey(&s.hotkey).into());
            ui.set_max_items(s.max_items as i32);
            ui.set_language(LANGUAGES.iter().position(|l| *l == s.language).unwrap_or(0) as i32);
            ui.set_color_scheme_index(theme_index(&s.theme));
        }
        ui.set_error(0);
        ui.set_autostart(autostart::is_enabled());
        ui.set_version(env!("CARGO_PKG_VERSION").into());
        fill_ocr(&ui, &self.ocr_status.borrow());
        // The user may have installed it since the last check; the answer updates the dialog.
        self.check_ocr();
        ui.invoke_show_settings();
    }

    fn set_theme(&self, index: i32) {
        let mut s = self.settings.borrow_mut();
        s.theme = THEMES.get(index as usize).copied().unwrap_or("").into();
        s.save();
        self.ui().set_color_scheme_index(index);
    }

    /// Switches the language in place: the interface, the tray menu and the automatic OCR languages follow it.
    fn set_language(&self, index: i32) {
        {
            let mut s = self.settings.borrow_mut();
            s.language = LANGUAGES.get(index as usize).copied().unwrap_or("").into();
            s.save();
            crate::apply_language(&s);
        }
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray::set_labels(tray, tray_labels(&self.ui()));
        }
        self.refresh();
        self.check_ocr();
    }

    /// The cap is read by the next copy; entries beyond a lower cap go now.
    fn set_max_items(&self, value: i32) {
        let Some(max_items) = usize::try_from(value).ok().filter(|n| MAX_ITEMS_RANGE.contains(n)) else {
            return;
        };
        {
            let mut s = self.settings.borrow_mut();
            s.max_items = max_items;
            s.save();
        }
        self.max_items.store(max_items, Ordering::SeqCst);
        {
            let mut h = self.history.lock().unwrap();
            h.set_max(max_items);
            h.save();
        }
        self.refresh();
    }

    /// A key press while the Shortcut button records. True ends the recording: the new shortcut is grabbed first and
    /// only then replaces the old one, so a taken key (`error` 2) keeps the shortcut that works.
    fn record_key(&self, text: &str, modifiers: [bool; 4]) -> bool {
        let mut text = text.to_string();
        if let Some(c) = text.chars().next().filter(|_| text.chars().count() == 1)
            && let Some(base) = self.x.base_key(c)
        {
            text = base.to_string();
        }
        let Some(hotkey) = shortcut_from_key(&text, modifiers) else {
            return false;
        };
        let ui = self.ui();
        let hotkey = hotkey.trim().to_lowercase();
        if x11::parse_hotkey(&hotkey).is_none() {
            ui.set_error(1);
        } else if let Err(e) = self.x.set_hotkey(&hotkey) {
            eprintln!("{e}");
            ui.set_error(2);
        } else {
            ui.set_error(0);
            ui.set_hotkey(shown_hotkey(&hotkey).into());
            let mut s = self.settings.borrow_mut();
            s.hotkey = hotkey;
            s.save();
        }
        true
    }

    fn paste_payload(&self, payload: Payload) {
        match payload {
            Payload::Text(text, html) => self.x.set_clipboard(text, html),
            Payload::Image(png) => self.x.set_clipboard_image(png),
        }
        self.hide();
        self.paste_attempts.set(0);
        self.paste_timer
            .start(slint::TimerMode::Repeated, PASTE_POLL, tick(App::paste_tick));
    }

    /// Paste once the window that was in front before the popup has focus again. Pasting on a fixed delay fails
    /// whenever the window manager is slower than the delay.
    fn paste_tick(&self) {
        let attempts = self.paste_attempts.get() + 1;
        self.paste_attempts.set(attempts);
        let previous = self.previous.get();
        let active = self.x.active_window();
        let ready = match previous {
            // Give the application a moment to react to getting focus back.
            Some(window) => active == Some(window) && attempts >= 3 && self.x.input_focus_inside(window),
            None => attempts >= 6,
        };
        if !ready && attempts == 8 {
            // The WM did not restore focus by itself.
            if let Some(window) = previous {
                debug(|| format!("restoring focus to {window:#x} (active {active:?})"));
                self.x.focus(window);
            }
        }
        if ready || attempts >= PASTE_MAX_POLLS {
            self.paste_timer.stop();
            // On timeout the previous window may be gone or the user went elsewhere: keep the text on the
            // clipboard instead of sending Ctrl+V to an unrelated window.
            if previous.is_none() || active == previous {
                let result = self.x.paste(active.is_some_and(|w| self.x.is_terminal(w)));
                debug(|| format!("paste after {attempts} polls, active {active:?}, previous {previous:?}: {result:?}"));
                if let Err(e) = result {
                    eprintln!("paste failed: {e}");
                }
            } else {
                eprintln!(
                    "paste skipped: {previous:?} did not regain focus (active {active:?}); it is on the clipboard"
                );
            }
        }
    }

    /// Alt+D: deletes the row from the history and keeps the selection on the same position.
    fn remove(&self, row: i32) {
        debug(|| format!("remove row {row}"));
        {
            let mut h = self.history.lock().unwrap();
            let Some(idx) = self.resolve_row(&h, row) else {
                return;
            };
            h.remove(idx);
            h.save();
        }
        self.refresh();
        let last = self.view.borrow().rows.len() as i32 - 1;
        self.ui().set_current(row.min(last).max(0));
    }
}

const MODIFIER_NAMES: [&str; 4] = ["ctrl", "alt", "shift", "super"];

/// "ctrl+alt+v" from a key press (`mods` is [ctrl, alt, shift, meta]), or None for a lone modifier or a key the
/// hotkey parser does not accept: one letter or digit, and at least one modifier (a bare key would steal typing).
fn shortcut_from_key(text: &str, mods: [bool; 4]) -> Option<String> {
    let mut chars = text.chars();
    let key = chars
        .next()
        .filter(|c| chars.next().is_none() && c.is_ascii_alphanumeric())?
        .to_ascii_lowercase();
    let mut parts: Vec<String> = MODIFIER_NAMES
        .iter()
        .zip(mods)
        .filter(|(_, on)| *on)
        .map(|(n, _)| n.to_string())
        .collect();
    if parts.is_empty() {
        return None;
    }
    parts.push(key.to_string());
    Some(parts.join("+"))
}

/// The stored shortcut ("ctrl+alt+v") as shown in Preferences ("Ctrl+Alt+V"); the stored value stays as it is.
fn shown_hotkey(stored: &str) -> String {
    stored
        .split('+')
        .map(|p| {
            let mut c = p.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join("+")
}

/// Live label while recording: "Ctrl + Alt + …", or the prompt when nothing is held.
fn held_text(mods: [bool; 4]) -> String {
    let held: Vec<&str> = ["Ctrl", "Alt", "Shift", "Super"]
        .into_iter()
        .zip(mods)
        .filter(|(_, on)| *on)
        .map(|(n, _)| n)
        .collect();
    if held.is_empty() {
        "Press keys… (Esc cancels)".into()
    } else {
        format!("{} + …", held.join(" + "))
    }
}

/// A list row is one line: whitespace and newlines collapse, and very long text is cut (the full text is still pasted).
/// Stops after 200 characters: a search builds a row for every match, and an entry can be 1 MiB.
fn one_line(text: &str) -> String {
    let (mut out, mut n, mut gap) = (String::new(), 0, false);
    for c in text.chars() {
        if c.is_whitespace() {
            gap = n > 0; // leading whitespace is dropped, a run of it becomes one space
            continue;
        }
        if n >= 200 {
            break;
        }
        if gap {
            out.push(' ');
            n += 1;
            gap = false;
            if n >= 200 {
                break;
            }
        }
        out.push(c);
        n += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{THEMES, View, one_line, resolve, shortcut_from_key, shown_hotkey, theme_index};
    use fastpaste_core::history::History;

    #[test]
    fn shortcut_is_shown_capitalised() {
        assert_eq!(shown_hotkey("ctrl+alt+v"), "Ctrl+Alt+V");
        assert_eq!(shown_hotkey("super+F5"), "Super+F5");
        assert_eq!(shown_hotkey(""), "");
    }

    #[test]
    fn themes_and_their_indexes_agree() {
        for (i, t) in THEMES.iter().enumerate() {
            assert_eq!(theme_index(t), i as i32);
        }
        assert_eq!(theme_index("bogus"), 0);
    }

    #[test]
    fn a_row_resolves_only_while_the_list_matches_the_history() {
        let mut h = History::default();
        h.push("a".into(), None, 10);
        let view = View {
            rows: vec![0],
            rev: h.rev(),
            ..Default::default()
        };
        assert_eq!(resolve(&view, &h, 0), Some(0));
        assert_eq!(resolve(&view, &h, 1), None, "no such row");
        assert_eq!(resolve(&view, &h, -1), None);
        h.push("b".into(), None, 10); // a copy while the popup is open: row 0 is now another item
        assert_eq!(resolve(&view, &h, 0), None);
    }

    #[test]
    fn recorded_shortcuts() {
        assert_eq!(
            shortcut_from_key("V", [true, true, false, false]).as_deref(),
            Some("ctrl+alt+v")
        );
        assert_eq!(
            shortcut_from_key("7", [false, false, true, true]).as_deref(),
            Some("shift+super+7")
        );
        assert_eq!(shortcut_from_key("v", [false; 4]), None, "needs a modifier");
        assert_eq!(
            shortcut_from_key("\u{f704}", [true; 4]),
            None,
            "F-keys are not accepted"
        );
        assert_eq!(shortcut_from_key("ab", [true; 4]), None);
    }

    #[test]
    fn rows_are_single_line_and_bounded() {
        assert_eq!(one_line("a\n\n  b\tc"), "a b c");
        assert_eq!(one_line(&"x".repeat(500)).len(), 200);
        assert_eq!(one_line("  \n lead and trail \t\n"), "lead and trail");
        assert_eq!(
            one_line(&format!("{} b", "é".repeat(199))).chars().count(),
            200,
            "cut by characters"
        );
        assert_eq!(one_line(&format!("{} b", "é".repeat(200))).chars().count(), 200);
    }
}

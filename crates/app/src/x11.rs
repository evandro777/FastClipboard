//! X11 side. One thread watches the clipboard (XFixes, no polling), serves it while we own it and listens for the
//! global hotkey; `Handle` is what the UI thread uses for clipboard ownership, focus hand-over and the synthetic paste.
//! Focus and paste handling follows FastMoji, where it was tested on Cinnamon (Muffin).
use std::{
    error::Error,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use x11rb::{
    COPY_FROM_PARENT, CURRENT_TIME, NONE,
    connection::Connection,
    protocol::{
        Event,
        xfixes::{ConnectionExt as _, SelectionEventMask},
        xproto::*,
        xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Same caps as `History`: a bigger property is skipped instead of truncated.
const MAX_PROP_UNITS: u32 = (fastpaste_core::history::MAX_ITEM_BYTES / 4) as u32;
const MAX_IMAGE_UNITS: u32 = (fastpaste_core::history::MAX_IMAGE_BYTES / 4) as u32;
const XK_SHIFT_L: u32 = 0xffe1;
const XK_CONTROL_L: u32 = 0xffe3;
const XK_V: u32 = 0x76;
/// Shift, Control, Alt and Super, left and right: released before a paste if physically down.
const MODIFIER_KEYSYMS: [u32; 8] = [0xffe1, 0xffe2, 0xffe3, 0xffe4, 0xffe9, 0xffea, 0xffeb, 0xffec];
/// Presses closer together than this are key auto-repeat (the shortcut held down), not new presses.
const REPEAT_MS: u32 = 100;

struct Atoms {
    clipboard: Atom,
    utf8: Atom,
    html: Atom,
    png: Atom,
    jpeg: Atom,
    targets: Atom,
    incr: Atom,
    /// Password managers add this target to what they put on the clipboard.
    secret_hint: Atom,
    prop: Atom,
    active: Atom,
    user_time: Atom,
}

/// What we serve while we own the clipboard: text (empty for an image), its HTML when the source had one, or a PNG.
struct Content {
    text: String,
    html: Option<String>,
    png: Option<Vec<u8>>,
}

type Own = Arc<Mutex<Option<Content>>>;

#[derive(Clone)]
pub struct Handle {
    conn: Arc<RustConnection>,
    root: Window,
    win: Window,
    a: Arc<Atoms>,
    own: Own,
    /// (modifiers, keycode) of the grabbed shortcut; swapped by `set_hotkey` while the daemon thread reads it.
    hotkey: Arc<Mutex<(u16, u8)>>,
}

/// "ctrl+alt+v" -> (modifier mask, keysym). The key is one ASCII letter or digit, whose keysym is its char code.
pub fn parse_hotkey(s: &str) -> Option<(u16, u32)> {
    let (mut mods, mut key) = (0u16, None);
    for part in s.split('+').map(|p| p.trim().to_lowercase()) {
        match part.as_str() {
            "ctrl" | "control" => mods |= u16::from(ModMask::CONTROL),
            "alt" => mods |= u16::from(ModMask::M1),
            "shift" => mods |= u16::from(ModMask::SHIFT),
            "super" => mods |= u16::from(ModMask::M4),
            k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => key = Some(u32::from(k.as_bytes()[0])),
            _ => return None,
        }
    }
    key.filter(|_| mods != 0).map(|k| (mods, k))
}

/// The grab is exact, so Caps Lock and Num Lock would silently disable it unless every combination is grabbed.
fn lock_combinations() -> [u16; 4] {
    [
        0,
        u16::from(ModMask::LOCK),
        u16::from(ModMask::M2),
        u16::from(ModMask::LOCK | ModMask::M2),
    ]
}

impl Handle {
    /// Grabs `hotkey` for every lock combination; on failure nothing stays grabbed.
    fn grab(&self, hotkey: &str) -> std::result::Result<(u16, u8), String> {
        let (mods, sym) = parse_hotkey(hotkey).ok_or_else(|| format!("invalid hotkey \"{hotkey}\""))?;
        let code = keycodes(&*self.conn, &[sym]).map_err(|e| format!("hotkey \"{hotkey}\": {e}"))?[0];
        for lock in lock_combinations() {
            let grab = self
                .conn
                .grab_key(
                    true,
                    self.root,
                    ModMask::from(mods | lock),
                    code,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .map_err(|e| format!("X11: {e}"))?;
            if grab.check().is_err() {
                self.ungrab(mods, code);
                return Err(format!(
                    "hotkey \"{hotkey}\" is already in use (is another instance running?)"
                ));
            }
        }
        Ok((mods, code))
    }

    fn ungrab(&self, mods: u16, code: u8) {
        for lock in lock_combinations() {
            let _ = self.conn.ungrab_key(code, self.root, ModMask::from(mods | lock));
        }
        let _ = self.conn.flush();
    }

    /// Moves the grab to `hotkey` (the first grab after `start`, too). The old one stays when the new one is taken by
    /// another program.
    pub fn set_hotkey(&self, hotkey: &str) -> std::result::Result<(), String> {
        let (mods, code) = self.grab(hotkey)?;
        let (old_mods, old_code) = std::mem::replace(&mut *self.hotkey.lock().unwrap(), (mods, code));
        // Keycode 0 is "nothing grabbed yet", and ungrabbing it would mean AnyKey.
        if old_code != 0 && (old_mods, old_code) != (mods, code) {
            self.ungrab(old_mods, old_code);
        }
        Ok(())
    }

    /// The unshifted character on the physical key that produces `c` in the current layout (`{` -> `7` where AltGr+7
    /// gives `{`; Ctrl+Alt is AltGr there), so the shortcut recorder names keys regardless of Ctrl/Alt/Shift.
    pub fn base_key(&self, c: char) -> Option<char> {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = self.conn.get_keyboard_mapping(min, max - min + 1).ok()?.reply().ok()?;
        let per = map.keysyms_per_keycode as usize;
        let first = map.keysyms.chunks(per).find(|k| k.contains(&(c as u32)))?[0];
        char::from_u32(first).filter(char::is_ascii_graphic)
    }
}

/// Keycodes for `keysyms`, from one fetch of the keyboard map.
fn keycodes(conn: &impl Connection, keysyms: &[u32]) -> Result<Vec<u8>> {
    keysyms
        .iter()
        .zip(find_keycodes(conn, keysyms)?)
        .map(|(keysym, code)| code.ok_or_else(|| format!("no key for keysym {keysym:#x}").into()))
        .collect()
}

/// Like `keycodes`, with `None` for a symbol the layout has no key for.
fn find_keycodes(conn: &impl Connection, keysyms: &[u32]) -> Result<Vec<Option<u8>>> {
    let setup = conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let map = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
    let per = usize::from(map.keysyms_per_keycode);
    let chunks = || map.keysyms.chunks(per);
    Ok(keysyms
        .iter()
        .map(|keysym| {
            // Prefer a key that produces the symbol unshifted in the first layout group.
            chunks()
                .position(|c| c.first() == Some(keysym))
                .or_else(|| chunks().position(|c| c.contains(keysym)))
                .map(|i| min + i as u8)
        })
        .collect())
}

/// True when a press at `time` (server ms, wraps) follows the previous one so closely that it is auto-repeat.
fn is_repeat(last: Option<u32>, time: u32) -> bool {
    last.is_some_and(|t| time.wrapping_sub(t) < REPEAT_MS)
}

/// Terminals that paste with Ctrl+Shift+V (plain Ctrl+V reaches the shell as a control character).
const TERMINAL_CLASSES: [&str; 10] = [
    "term",
    "konsole",
    "kitty",
    "alacritty",
    "wezterm",
    "foot",
    "ptyxis",
    "guake",
    "yakuake",
    "sakura",
];

/// Does a `WM_CLASS` value (instance and class, any case) belong to a terminal emulator?
fn is_terminal_class(class: &str) -> bool {
    let class = class.to_ascii_lowercase();
    TERMINAL_CLASSES.iter().any(|t| class.contains(t))
}

/// Firefox offers text/html as UTF-16LE with a BOM; everything else is UTF-8.
fn decode_html(b: &[u8]) -> String {
    if let [0xff, 0xfe, rest @ ..] = b {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(b).into_owned()
}

/// What we are fetching from the current clipboard owner: its target list (to spot a password manager and to tell
/// text from an image), then either the plain text and its HTML flavor, or the PNG.
enum Stage {
    Idle,
    Targets,
    Text,
    Html(String),
    Image,
    /// The owner sends big data in chunks (INCR): each chunk arrives as a property change on our window and ends with
    /// an empty one. Starts well below the size caps (about 256 KB), so text and HTML go through it too.
    Incr(Incr),
}

/// What an INCR transfer is carrying, and what to do with it once it ends.
enum Flavor {
    Text,
    /// The HTML flavor of this plain text.
    Html(String),
    Image,
}

struct Incr {
    flavor: Flavor,
    /// `None` once it grew past the cap: the rest is read and thrown away so the owner can finish.
    buf: Option<Vec<u8>>,
}

/// What a finished transfer leads to.
#[derive(Debug, PartialEq)]
enum Done {
    /// The plain text arrived: ask for its HTML next.
    Text(String),
    Copy(String, Option<String>),
    Image(Vec<u8>),
    Nothing,
}

impl Incr {
    fn new(flavor: Flavor) -> Self {
        Incr {
            flavor,
            buf: Some(Vec::new()),
        }
    }

    fn add(&mut self, chunk: &[u8]) {
        let cap = match self.flavor {
            Flavor::Image => fastpaste_core::history::MAX_IMAGE_BYTES,
            _ => fastpaste_core::history::MAX_ITEM_BYTES,
        };
        if let Some(all) = &mut self.buf {
            if all.len() + chunk.len() > cap {
                self.buf = None;
            } else {
                all.extend_from_slice(chunk);
            }
        }
    }

    /// Too big text or image: dropped. Too big HTML: the text is still recorded, without it.
    fn finish(self) -> Done {
        match (self.flavor, self.buf) {
            (Flavor::Text, Some(b)) => Done::Text(String::from_utf8_lossy(&b).into_owned()),
            (Flavor::Html(text), b) => Done::Copy(text, b.map(|b| decode_html(&b))),
            (Flavor::Image, Some(png)) => Done::Image(png),
            _ => Done::Nothing,
        }
    }
}

struct Daemon {
    h: Handle,
    last_press: Option<u32>,
    stage: Stage,
    /// The image type requested in `Stage::Image` (PNG, or JPEG when that is all the owner offers).
    image_target: Atom,
    on_copy: Box<dyn Fn(String, Option<String>) + Send>,
    on_image: Box<dyn Fn(Vec<u8>) + Send>,
    on_hotkey: Box<dyn Fn(u32) + Send>,
}

/// Fails when the display is unreachable. No shortcut is grabbed yet: the caller does that with `set_hotkey`, so a
/// taken shortcut is an error to show, not the end of the app.
/// `on_hotkey` gets the event time, which the focus hand-over needs. `on_image` gets the bytes of an image copy (PNG or JPEG).
pub fn start(
    on_copy: impl Fn(String, Option<String>) + Send + 'static,
    on_image: impl Fn(Vec<u8>) + Send + 'static,
    on_hotkey: impl Fn(u32) + Send + 'static,
) -> std::result::Result<Handle, String> {
    let setup = || -> Result<Handle> {
        let (conn, screen) = RustConnection::connect(None)?;
        let root = conn.setup().roots[screen].root;
        let win = conn.generate_id()?;
        conn.create_window(
            0,
            win,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            COPY_FROM_PARENT,
            // PROPERTY_CHANGE: the chunks of an INCR transfer are announced that way.
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;
        let atom = |name: &[u8]| -> Result<Atom> { Ok(conn.intern_atom(false, name)?.reply()?.atom) };
        let a = Arc::new(Atoms {
            clipboard: atom(b"CLIPBOARD")?,
            utf8: atom(b"UTF8_STRING")?,
            html: atom(b"text/html")?,
            png: atom(b"image/png")?,
            jpeg: atom(b"image/jpeg")?,
            targets: atom(b"TARGETS")?,
            incr: atom(b"INCR")?,
            secret_hint: atom(b"x-kde-passwordManagerHint")?,
            prop: atom(b"_FASTPASTE_PROP")?,
            active: atom(b"_NET_ACTIVE_WINDOW")?,
            user_time: atom(b"_NET_WM_USER_TIME")?,
        });
        conn.xfixes_query_version(5, 0)?.reply()?;
        conn.xfixes_select_selection_input(win, a.clipboard, SelectionEventMask::SET_SELECTION_OWNER)?;
        Ok(Handle {
            conn: Arc::new(conn),
            root,
            win,
            a,
            own: Arc::new(Mutex::new(None)),
            hotkey: Arc::new(Mutex::new((0, 0))),
        })
    };
    let h = setup().map_err(|e| format!("X11: {e}"))?;

    let daemon = Daemon {
        h: h.clone(),
        last_press: None,
        stage: Stage::Idle,
        image_target: h.a.png,
        on_copy: Box::new(on_copy),
        on_image: Box::new(on_image),
        on_hotkey: Box::new(on_hotkey),
    };
    thread::spawn(move || daemon.run());
    Ok(h)
}

impl Daemon {
    fn run(mut self) {
        let _ = self.h.conn.flush();
        while let Ok(event) = self.h.conn.wait_for_event() {
            match event {
                // Our own copy (a pick) also raises this; only other owners are recorded.
                Event::XfixesSelectionNotify(e) if e.selection == self.h.a.clipboard && e.owner != self.h.win => {
                    self.stage = Stage::Targets;
                    self.request(self.h.a.targets);
                }
                Event::SelectionNotify(e) => self.on_reply(e.target, e.property),
                Event::PropertyNotify(e)
                    if e.window == self.h.win && e.atom == self.h.a.prop && e.state == Property::NEW_VALUE =>
                {
                    self.on_chunk();
                }
                Event::SelectionRequest(e) => self.serve(&e),
                Event::SelectionClear(_) => *self.h.own.lock().unwrap() = None,
                Event::KeyPress(e) if e.detail == self.h.hotkey.lock().unwrap().1 => {
                    if !is_repeat(self.last_press, e.time) {
                        (self.on_hotkey)(e.time);
                    }
                    self.last_press = Some(e.time);
                }
                _ => {}
            }
            let _ = self.h.conn.flush();
        }
    }

    fn request(&self, target: Atom) {
        let h = &self.h;
        let _ = h
            .conn
            .convert_selection(h.win, h.a.clipboard, target, h.a.prop, CURRENT_TIME);
    }

    /// Reads (and deletes) the reply property. Deleting an INCR marker is what asks the owner for the first chunk.
    fn get(&self, prop: Atom, max_units: u32) -> Option<GetPropertyReply> {
        if prop == NONE {
            return None;
        }
        self.h
            .conn
            .get_property(true, self.h.win, prop, AtomEnum::ANY, 0, max_units)
            .ok()?
            .reply()
            .ok()
    }

    /// The target list the owner offers; `None` when it does not answer TARGETS.
    fn read_targets(&self, prop: Atom) -> Option<Vec<Atom>> {
        if prop == NONE {
            return None;
        }
        let r = self
            .h
            .conn
            .get_property(true, self.h.win, prop, AtomEnum::ATOM, 0, 1024)
            .ok()?
            .reply()
            .ok()?;
        Some(r.value32()?.collect())
    }

    fn on_reply(&mut self, target: Atom, prop: Atom) {
        match std::mem::replace(&mut self.stage, Stage::Idle) {
            Stage::Targets if target == self.h.a.targets => {
                // A copy from a password manager is never recorded. An owner that does not answer TARGETS is
                // treated as ordinary text.
                let targets = self.read_targets(prop).unwrap_or_default();
                let a = &self.h.a;
                if targets.contains(&a.secret_hint) {
                    return;
                }
                // Text wins when both are offered (a spreadsheet cell comes with a picture of itself); a PNG without
                // text is an image copy (a screenshot, "Copy image" in a browser). PNG is preferred; a JPEG is
                // converted to PNG by `images::to_png`.
                let image = [a.png, a.jpeg].into_iter().find(|t| targets.contains(t));
                if let Some(image) = image.filter(|_| !targets.contains(&a.utf8)) {
                    self.stage = Stage::Image;
                    self.image_target = image;
                    self.request(image);
                } else {
                    self.stage = Stage::Text;
                    self.request(a.utf8);
                }
            }
            Stage::Text if target == self.h.a.utf8 => {
                let Some(reply) = self.get(prop, MAX_PROP_UNITS) else {
                    return;
                };
                if reply.type_ == self.h.a.incr {
                    self.stage = Stage::Incr(Incr::new(Flavor::Text));
                } else if reply.bytes_after == 0 {
                    self.stage = Stage::Html(String::from_utf8_lossy(&reply.value).into_owned());
                    self.request(self.h.a.html);
                }
            }
            Stage::Html(text) if target == self.h.a.html => match self.get(prop, MAX_PROP_UNITS) {
                Some(r) if r.type_ == self.h.a.incr => self.stage = Stage::Incr(Incr::new(Flavor::Html(text))),
                Some(r) if r.bytes_after == 0 => (self.on_copy)(text, Some(decode_html(&r.value))),
                // No HTML flavor, or too big for one property: the text alone is still worth recording.
                _ => (self.on_copy)(text, None),
            },
            Stage::Image if target == self.image_target => {
                let Some(reply) = self.get(prop, MAX_IMAGE_UNITS) else {
                    return;
                };
                if reply.type_ == self.h.a.incr {
                    self.stage = Stage::Incr(Incr::new(Flavor::Image));
                } else if reply.bytes_after == 0 {
                    (self.on_image)(reply.value);
                }
            }
            other => self.stage = other, // a late reply to a request that was already replaced
        }
    }

    /// One chunk of an INCR transfer. Reading it (which deletes the property) asks for the next; an empty one is the end.
    fn on_chunk(&mut self) {
        if !matches!(self.stage, Stage::Incr(_)) {
            return; // the property changed for another reason (a plain reply being written)
        }
        let Some(chunk) = self.get(self.h.a.prop, MAX_IMAGE_UNITS) else {
            self.stage = Stage::Idle; // never record half of it
            return;
        };
        if !chunk.value.is_empty() {
            if let Stage::Incr(incr) = &mut self.stage {
                incr.add(&chunk.value);
            }
            return;
        }
        let Stage::Incr(done) = std::mem::replace(&mut self.stage, Stage::Idle) else {
            return;
        };
        match done.finish() {
            Done::Text(text) => {
                self.stage = Stage::Html(text);
                self.request(self.h.a.html);
            }
            Done::Copy(text, html) => (self.on_copy)(text, html),
            Done::Image(png) => (self.on_image)(png),
            Done::Nothing => {}
        }
    }

    fn serve(&self, e: &SelectionRequestEvent) {
        let prop = if e.property == NONE { e.target } else { e.property };
        let (a, c) = (&self.h.a, &self.h.conn);
        let own = self.h.own.lock().unwrap();
        let done = match &*own {
            Some(content) if e.selection == a.clipboard => {
                let has_text = !content.text.is_empty();
                let is_string = e.target == a.utf8 || e.target == Atom::from(AtomEnum::STRING);
                if e.target == a.targets {
                    let mut t = vec![a.targets];
                    if has_text {
                        t.extend([a.utf8, AtomEnum::STRING.into()]);
                    }
                    t.extend(content.html.is_some().then_some(a.html));
                    t.extend(content.png.is_some().then_some(a.png));
                    c.change_property32(PropMode::REPLACE, e.requestor, prop, AtomEnum::ATOM, &t)
                        .is_ok()
                } else if is_string && has_text {
                    c.change_property8(PropMode::REPLACE, e.requestor, prop, e.target, content.text.as_bytes())
                        .is_ok()
                } else if let (true, Some(html)) = (e.target == a.html, &content.html) {
                    c.change_property8(PropMode::REPLACE, e.requestor, prop, e.target, html.as_bytes())
                        .is_ok()
                } else if let (true, Some(png)) = (e.target == a.png, &content.png) {
                    c.change_property8(PropMode::REPLACE, e.requestor, prop, e.target, png)
                        .is_ok()
                } else {
                    false
                }
            }
            _ => false,
        };
        let reply = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: e.time,
            requestor: e.requestor,
            selection: e.selection,
            target: e.target,
            property: if done { prop } else { NONE },
        };
        let _ = c.send_event(false, e.requestor, EventMask::NO_EVENT, reply);
    }
}

impl Handle {
    /// Becomes the clipboard owner with this content. Synchronous, so a paste right after it sees the new content.
    pub fn set_clipboard(&self, text: String, html: Option<String>) {
        self.own_clipboard(Content { text, html, png: None });
    }

    /// Same for an image copy: only `image/png` is offered.
    pub fn set_clipboard_image(&self, png: Vec<u8>) {
        self.own_clipboard(Content {
            text: String::new(),
            html: None,
            png: Some(png),
        });
    }

    fn own_clipboard(&self, content: Content) {
        *self.own.lock().unwrap() = Some(content);
        let _ = self.conn.set_selection_owner(self.win, self.a.clipboard, CURRENT_TIME);
        let _ = self.conn.flush();
    }

    /// The window manager's currently active top-level window.
    pub fn active_window(&self) -> Option<u32> {
        let reply = self
            .conn
            .get_property(false, self.root, self.a.active, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        reply.value32().and_then(|mut v| v.next()).filter(|w| *w != 0)
    }

    /// True when the X input focus is `window` or one of its descendants. `_NET_ACTIVE_WINDOW` can
    /// change slightly before the real keyboard focus does, and a key sent in between is lost.
    pub fn input_focus_inside(&self, window: u32) -> bool {
        let walk = || -> Result<bool> {
            let mut current = self.conn.get_input_focus()?.reply()?.focus;
            for _ in 0..16 {
                if current == window {
                    return Ok(true);
                }
                if current == self.root || current == 0 {
                    break;
                }
                current = self.conn.query_tree(current)?.reply()?.parent;
            }
            Ok(false)
        };
        walk().unwrap_or(true)
    }

    fn request_active(&self, window: u32, data: [u32; 5]) -> Result<()> {
        let message = ClientMessageEvent::new(32, window, self.a.active, data);
        self.conn.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            message,
        )?;
        self.conn.flush()?;
        Ok(())
    }

    /// Ask the WM to focus `window` the way a pager would (not subject to focus-stealing prevention).
    pub fn focus(&self, window: u32) {
        let _ = self.request_active(window, [2, 0, 0, 0, 0]);
    }

    /// Hand focus to our popup with the hotkey's timestamp, or Muffin treats the request as focus stealing and
    /// ignores a plain show().
    pub fn activate(&self, window: u32, time: u32) -> Result<()> {
        self.conn
            .change_property32(PropMode::REPLACE, window, self.a.user_time, AtomEnum::CARDINAL, &[time])?;
        self.conn.set_input_focus(InputFocus::PARENT, window, time)?;
        self.request_active(window, [1, time, 0, 0, 0])
    }

    /// Tells the WM where to map `window` (USPosition/PPosition in `WM_NORMAL_HINTS`). A WM places a new window
    /// itself unless the position is a hint set before mapping; moving it afterwards shows it at the WM's spot first.
    pub fn set_position_hint(&self, window: u32, x: i32, y: i32) {
        // WM_SIZE_HINTS: flags, x, y, width, height, ... 18 CARD32 in all; keep what the toolkit already put there.
        let mut hints: Vec<u32> = self
            .conn
            .get_property(false, window, AtomEnum::WM_NORMAL_HINTS, AtomEnum::WM_SIZE_HINTS, 0, 18)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(Iterator::collect))
            .unwrap_or_default();
        hints.resize(18, 0);
        hints[0] |= 1 | 4; // USPosition | PPosition
        hints[1] = x as u32;
        hints[2] = y as u32;
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_NORMAL_HINTS,
            AtomEnum::WM_SIZE_HINTS,
            &hints,
        );
        let _ = self.conn.flush();
    }

    /// The mouse pointer in root coordinates.
    pub fn pointer(&self) -> Option<(i32, i32)> {
        let p = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        Some((i32::from(p.root_x), i32::from(p.root_y)))
    }

    /// Size of the whole screen (all monitors together).
    pub fn screen_size(&self) -> Option<(i32, i32)> {
        let g = self.conn.get_geometry(self.root).ok()?.reply().ok()?;
        Some((i32::from(g.width), i32::from(g.height)))
    }

    /// True when `window` is a terminal emulator, judged by its `WM_CLASS`.
    pub fn is_terminal(&self, window: u32) -> bool {
        let check = || -> Result<bool> {
            let reply = self
                .conn
                .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 64)?
                .reply()?;
            Ok(is_terminal_class(&String::from_utf8_lossy(&reply.value)))
        };
        check().unwrap_or(false)
    }

    /// Send Ctrl+V (Ctrl+Shift+V when `terminal`) to whatever window has focus.
    pub fn paste(&self, terminal: bool) -> Result<()> {
        // Looked up per paste, not cached: the user may change keyboard layout while we run.
        let codes = keycodes(&*self.conn, &[XK_CONTROL_L, XK_SHIFT_L, XK_V])?;
        let (ctrl, shift, v) = (codes[0], codes[1], codes[2]);
        // The user is still holding Shift/Alt/Ctrl from the shortcut that picked the entry; the target would see
        // Ctrl+Shift+V (Paste Special in office apps, nothing in xed). Release them first.
        // Layouts without e.g. a right Alt simply have fewer of these.
        let mods = find_keycodes(&*self.conn, &MODIFIER_KEYSYMS)?;
        let down = self.conn.query_keymap()?.reply()?.keys;
        let mut keys: Vec<_> = mods
            .into_iter()
            .flatten()
            .filter(|&c| down[usize::from(c / 8)] & (1 << (c % 8)) != 0)
            .map(|c| (KEY_RELEASE_EVENT, c))
            .collect();
        keys.push((KEY_PRESS_EVENT, ctrl));
        if terminal {
            keys.push((KEY_PRESS_EVENT, shift));
        }
        keys.push((KEY_PRESS_EVENT, v));
        keys.push((KEY_RELEASE_EVENT, v));
        if terminal {
            keys.push((KEY_RELEASE_EVENT, shift));
        }
        keys.push((KEY_RELEASE_EVENT, ctrl));
        // One by one with a pause and a final round-trip: sent back to back, GTK3 apps lost about a quarter of pastes.
        for (kind, code) in keys {
            self.conn.xtest_fake_input(kind, code, 0, self.root, 0, 0, 0)?;
            self.conn.flush()?;
            thread::sleep(Duration::from_millis(8));
        }
        self.conn.get_input_focus()?.reply()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_parsing() {
        assert_eq!(
            parse_hotkey("Ctrl+Alt+V"),
            Some((u16::from(ModMask::CONTROL | ModMask::M1), u32::from(b'v')))
        );
        assert_eq!(parse_hotkey("v"), None); // a bare letter would steal typing everywhere
        assert_eq!(parse_hotkey("ctrl+alt"), None);
        assert_eq!(parse_hotkey("ctrl+f13"), None);
    }

    #[test]
    fn chunked_text_is_joined_and_html_is_decoded() {
        let mut t = Incr::new(Flavor::Text);
        t.add(b"hel");
        t.add("lo é".as_bytes());
        assert_eq!(t.finish(), Done::Text("hello é".into()));

        let mut h = Incr::new(Flavor::Html("hi".into()));
        h.add(b"<b>hi</b>");
        assert_eq!(h.finish(), Done::Copy("hi".into(), Some("<b>hi</b>".into())));
    }

    #[test]
    fn chunked_data_over_the_cap_is_dropped_but_the_text_is_kept() {
        let big = vec![b'x'; fastpaste_core::history::MAX_ITEM_BYTES / 2 + 1];
        let mut text = Incr::new(Flavor::Text);
        let mut html = Incr::new(Flavor::Html("t".into()));
        for _ in 0..2 {
            text.add(&big);
            html.add(&big);
        }
        assert_eq!(text.finish(), Done::Nothing);
        assert_eq!(
            html.finish(),
            Done::Copy("t".into(), None),
            "text alone is still recorded"
        );

        // The image cap is higher: the same 1 MiB passes.
        let mut image = Incr::new(Flavor::Image);
        image.add(&vec![0; fastpaste_core::history::MAX_ITEM_BYTES + 1]);
        assert!(matches!(image.finish(), Done::Image(_)));
    }

    #[test]
    fn html_from_firefox_is_utf16() {
        let mut b = vec![0xff, 0xfe];
        b.extend("<b>é</b>".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode_html(&b), "<b>é</b>");
        assert_eq!(decode_html("<i>é</i>".as_bytes()), "<i>é</i>");
    }

    #[test]
    fn recognises_terminal_classes() {
        assert!(is_terminal_class("gnome-terminal-server\0Gnome-terminal\0"));
        assert!(is_terminal_class("konsole\0konsole\0"));
        assert!(!is_terminal_class("xed\0Xed\0"));
        assert!(!is_terminal_class("firefox\0firefox\0"));
    }

    #[test]
    fn held_key_repeats_are_ignored() {
        assert!(!is_repeat(None, 5), "first press");
        assert!(is_repeat(Some(1000), 1030), "auto-repeat");
        assert!(!is_repeat(Some(1000), 1000 + REPEAT_MS), "a real second press");
        assert!(is_repeat(Some(u32::MAX - 10), 20), "across the timestamp wrap");
    }
}

# AGENTS.md: FastPaste

Notes for AI agents and contributors. Read this before changing anything.

## What this is

A **light and fast** Linux desktop app in Rust + Slint. Priorities: fast start, low memory, simplicity.

A minimal CopyQ-like clipboard history for X11: a background process records text and HTML copies, a global hotkey
(default Ctrl+Alt+V) opens a popup to search and paste, Shift+Enter opens an options menu (plain text, transforms).

- `crates/core/src/history.rs`: the list (dedupe, cap, search, `history.toml`). Whole-file rewrite per copy (500 items cap).
- `crates/app/src/x11.rs`: one thread with the XFixes watcher, hotkey grab and clipboard serving; `x11::Handle` (shared
  connection) is what the UI thread uses: clipboard ownership, focus hand-over, `paste`.
- `crates/app/src/app.rs`: list state and the popup lifecycle (the popup is created hidden and shown by the hotkey).
  The focus/close/paste sequencing is ported from FastMoji (`../FastMoji`, tested on Cinnamon); read its AGENTS.md
  "Hard-won gotchas" before touching the timings. Short version: a plain `show()` gets no keyboard focus from Muffin,
  so `activate` stamps `_NET_WM_USER_TIME`, calls `XSetInputFocus` and sends `_NET_ACTIVE_WINDOW` with the hotkey's
  event time, retried until a focus-in; focus-out closes the popup (after a 300 ms grace, and a toggle that arrives
  right after such a close is ignored); the paste waits until the previous window is active again and sends
  Ctrl+Shift+V to terminals. The popup is a normal decorated window: a frameless/always-on-top one never got focus.
- `FASTPASTE_DEBUG=1 fastpaste` logs open/focus/hide/paste decisions to stderr.
- Testing the X11 side: private Xvfb (`:98`) + `muffin --display=:98 --sm-disable` (the real WM, so focus behaves as
  on the desktop) + a temp `XDG_DATA_HOME`, with `xev` as the paste target (`stdbuf -oL xev`). Assert the display
  size from `xdpyinfo` before running `xclip`/`xdotool`, and never chain setup with `&&` where a failure would leave
  `DISPLAY` unset: a first version of this test hit the real session.
- `crates/app/src/tray.rs` (ksni, StatusNotifierItem) + `autostart.rs`: tray menu Open / Preferences / Quit
  (Start at login and Clear history live in the preferences). The tray icon is a pixmap decoded from
  `packaging/io.github.evandro777.FastPaste.png` (the SVG rendered at 64x64 with ImageMagick: redo it when the SVG
  changes), so it shows without a theme install. ksni is built with `async-io` + `blocking` (its default pulls tokio; keep it off). The tray is optional: when
  `spawn` fails (no watcher, no bus) the app logs and runs on. Menu labels come from the `Tr` global (translated on the
  UI thread, handed to the tray thread as strings). Careful when testing: with `DBUS_SESSION_BUS_ADDRESS` unset zbus
  still falls back to `$XDG_RUNTIME_DIR/bus`, i.e. the real session bus; point `XDG_RUNTIME_DIR` at an empty dir too.
- The popup is centered on the mouse pointer, clamped to the monitor under it (winit's `available_monitors`). The WM
  must be told the position BEFORE the window is mapped: `x11::set_position_hint` writes USPosition/PPosition into
  `WM_NORMAL_HINTS` and `set_position` is called before `show()`. Moving the window after `show()` makes Muffin map it
  at its own spot first (top-left, then its default placement) and the popup visibly jumps. Check this with
  `xev -root -event substructure` (one Configure per open, none after the Map); a position poller is too slow to see it.
  The size is saved on hide.
- A copy whose TARGETS include `x-kde-passwordManagerHint` is skipped (x11.rs `Stage::Targets`).
- Images: `image/png` without `UTF8_STRING` in TARGETS is an image copy (text wins when both are offered). The PNG is
  stored as `images/<hash>.png` plus a ~192x96 `<hash>.thumb.png` made once at copy time (`images.rs`, `png` crate,
  area-average downscale); the TOML only holds an `[items.image]` pointer, and the files are deleted with the entry
  (`History::truncate/remove/clear`). The list loads only the tiny thumbnails, never a full image; images over 20 Mpix
  get no thumbnail, over 8 MB are not stored. Big images, and big text/HTML, arrive by INCR (chunked, driven by
  `PropertyNotify` on our window, `Stage::Incr`, `Flavor` says what it carries): xclip uses it from about 256 KB, so
  test with a >1 MB PNG and a 300 KB text. Image decoding runs on one worker thread (`on_image` only sends the bytes),
  so the X11 thread keeps serving the clipboard and the hotkey. Serving a pick sends the PNG in
  one property (x11rb uses BIG-REQUESTS), no INCR on the sending side. Everything stored is PNG: an `image/jpeg`-only copy is decoded (`zune-jpeg`) and re-encoded by `images::to_png`
  (sniffed by magic number, capped at 20 Mpix); PNG is preferred when both are offered. `History::load` deletes
  files in `images/` that no entry references, but only when `history.toml` was read and parsed (never after a missing or
  corrupt file).
- Transforms (`core/transform.rs`) are picked from the Shift+Enter menu (`choose(row, option)`: 0 original, 1 plain,
  2.. transforms); they paste text only and are not added to the history, like a plain paste. `x11::paste` first releases
  any modifier the user is still holding (query_keymap): otherwise a Shift+Enter pick arrived as Ctrl+Shift+V (Paste Special /
  nothing in xed) and Alt shortcuts as Ctrl+Alt+V. Alt+letter shortcuts were dropped for this menu.
- Preferences: a second Slint window (`Preferences` in main.slint, snapshot page 1) opened by the burger button of the
  popup, the tray, `--prefs` or the `prefs` socket command. Save applies without a restart: `x11::Handle::set_hotkey`
  grabs the new key first and only then lets go of the old one (a taken key keeps the old shortcut), `max_items` is an
  `AtomicUsize` read by the copy threads, and the language is switched with `select_bundled_translation` plus new tray
  labels. The shortcut is typed as keys (FocusScope in the window, `record-key`; layout-independent through
  `Handle::base_key`); the grab swallows the current shortcut, so `App::toggle` treats it as "keep" while recording.
  Start at login (switch) and Clear history (with a confirmation overlay) act immediately; About is an overlay.
  `LANGUAGES` in app.rs must stay in the order of `languages` in main.slint. The window was not given the popup's
  focus hand-over (`activate`); check the keyboard focus under Muffin.
- OCR languages: `ocr_lang = ""` means the interface language first, then English (`ocr::auto_lang`, mapped by hand, only packs
  `tesseract --list-langs` reports); a non-empty value is used as is.
- OCR: the Shift+Enter menu depends on the row (`Row.is-image`): text rows get `text-options`, image rows get
  `image-options` (Original, then the same options as OCR + transform; same positions in both lists, so `choose` maps
  them alike). They run `tesseract stdin stdout -l <ocr_lang>` in a thread (`ocr.rs`) on the stored PNG and paste the
  (transformed) text; popup closes first, errors go to stderr. tesseract is optional and shelled out on purpose (no OCR
  crate: size and build deps).
- A taken or invalid `hotkey` does not end the app: `x11::start` grabs nothing, `app::run` calls `set_hotkey` and, on
  failure, opens the preferences with `error` 1 or 2. The list rows are history indexes tagged with `History::rev`
  (`View.rev`, `resolve`): a copy while the popup is open makes a pick/remove do nothing and rebuild the list instead
  of acting on another item. The data dir is 0700 and its files 0600 (`settings::create_private_dir/write_private`).
- One instance only: `instance.rs` (ported from FastMoji) listens on `$XDG_RUNTIME_DIR/fastpaste.sock`. A later start
  sends `show` (or `toggle` with `--toggle`) and exits, before Slint or X11 start; fixtures mode skips this so
  `ui-snapshot.sh` never pokes a real instance. When testing, point `XDG_RUNTIME_DIR` at a temp dir. Alt+digit is no longer bound, so it types in the search.

## Layout

```txt
Cargo.toml                workspace, release/fast profiles
crates/core/              UI-independent logic (settings.rs). No Slint here, so a CLI can reuse it
crates/app/               Slint UI + main.rs; build.rs compiles ui/ and bundles translations/
packaging/                .desktop, icon, metainfo (used by the AppImage and CI validation)
appimage/build.sh         AppImage + .zsync (host libs, not bundled); CI: appimage.yml
dev/gen_po.py             writes the .po files from one table
scripts/ui-snapshot.sh    renders pages on a private Xvfb with canned data (crates/app/src/fixtures.rs, env FASTPASTE_FIXTURES=empty|single|many, FASTPASTE_PAGE=<n>) to target/ui-snapshots/
flatpak/                  manifest, build.sh (vendors crates, builds outside the repo: `~/.cache/fastpaste-flatpak`), signing key (public half); CI: flatpak.yml
README.md, BUILD.md       what users read / build, settings, OCR and shortcut details (keep both in step with behaviour)
docs/                     landing page (`site/index.html`, deployed with the Flatpak remote), `screenshots/`, `FLATHUB.md`
```

## Commands

```bash
cargo build --profile fast    # iteration build, no LTO
cargo build --release         # thin LTO
cargo test                    # unit tests
```

System deps: `libfontconfig-dev libxkbcommon-dev libxkbcommon-x11-dev libx11-dev libxcb1-dev`. No GL/Skia/femtovg.

## Code conventions: Editing code

- Use the Edit tool for code changes. Don't splice files with sed, awk, python or heredocs through Bash.
- After editing Rust, run `cargo fmt && cargo clippy -- -D warnings && cargo test`. Re-read the surrounding block to confirm no braces, lines or whole components (e.g. pages) were deleted.
- After any change, also run `cargo build --profile fast` so the binary is built and cached for the owner's manual testing.

## Architecture decisions (inherited from FastMoji, measured there)

- **Software renderer only** (`winit` + `software`, forced in `main.rs` and in `Cargo.toml` features with `default-features = false`): ~27 MB RSS versus ~108 MB with the GL renderer.
- No async runtime. Scans/work in threads, results handed to the UI thread with `slint::invoke_from_event_loop`.
- Never hold a `RefCell` borrow while calling something that makes Slint re-read a model.
- Keep dependencies few and light.
- Slint notes: a `VerticalLayout` inside a `ScrollView` needs `alignment: start`, or a short list spreads its rows over the window; read-only `TextInput` (one per block) makes text selectable; use `content-y` (not `viewport-y`); a callback named `close` shadows a builtin (use `dismiss`); `FocusScope.capture-key-pressed` intercepts keys before `LineEdit`.

## Translations

- UI strings use `@tr()`; strings built in Rust go through the `Tr` global in `ui/main.slint`. Never `format!` user-visible text in Rust.
- `.po` files are generated by `dev/gen_po.py` (edit its table, run it, commit both). Slint's gettext context is the component name (`Main`, `Tr`). A new `@tr` string needs an entry in every language.
- Language = `settings.language` (`""` = system locale).

## Conventions

- Rust 2024, `rustfmt` max_width 120, clippy `-D warnings`. Comments explain *why*, not what.
- Settings are serde TOML with `#[serde(default)]`; add a unit test for each new option so old files keep loading.
- Settings path: `$XDG_DATA_HOME/FastPaste/settings.toml`.
- Update `README.md` when behaviour changes, and this file when you learn something non-obvious.

## Testing

- `cargo test`. Manual harnesses (if any) go in `dev/`, must never capture the whole screen, and must never click or type at hard-coded coordinates.
- Never `pkill -f` with a pattern that appears in your own command line; kill by PID.

### Verifying UI changes

- After any Slint UI change, run `scripts/ui-snapshot.sh <page>` and read the PNGs (empty, single, many) before saying the change is done. It uses a private Xvfb and canned data, so it never reads or changes the real system.
- Specifically check: rows don't stretch when a list has 1 item, badges and buttons are right-aligned, icons are sharp, text is selectable as a block, and empty or error states (e.g. "nothing found") don't throw errors.
- If you can't launch it (e.g. the flow needs pkexec or a live X11 session), say so explicitly and list what I should test manually.

## Screenshots (README, landing page, store listings)

- Public screenshots must never show personal data: the owner's e-mail, name, username, home path, hostnames, or anything
  from the real clipboard history. A screenshot of the real desktop or the real history is forbidden.
- Make them only from `scripts/ui-snapshot.sh` (private Xvfb + canned data from `fixtures.rs`), then crop to the window.
  Fixture strings must be obviously fake (`jane.doe@example.com`, `/home/user/...`): never put real data in `fixtures.rs`.
- Before saving into `docs/screenshots/`, READ the final PNG and check every visible string. Never take the owner's word
  or your memory that it is clean. Check also the alt text and metadata (`strings file.png`).
- Once a file was committed or pushed it stays in git history; tell the owner if a leaked image needs a history rewrite.

## GitHub flow and releases

Work on branches, PRs to `master`. CI: `ci.yml` (fmt, clippy, tests, MSRV, metainfo/desktop validation), `appimage.yml` (AppImage on every push/PR; on a `v*` tag it attaches to the release), `virustotal.yml` (scans release assets; needs the `VIRUSTOTAL_API_KEY` secret). `RELEASING.md` has the steps. Never commit or push on the owner's behalf.

## Flatpak

App id `io.github.evandro777.FastPaste`; `flatpak/build.sh --devel` installs `...FastPaste.Devel` next to a release. Needs `flatpak-builder` and
`org.freedesktop.Sdk//25.08` + `org.freedesktop.Sdk.Extension.rust-stable//25.08`. **Not built or run yet**: verify the tray, hotkey, paste and the
socket inside the sandbox first.

- Permissions: `--share=ipc`, `--socket=x11` (X11 has no sandbox, so XFixes/XGrabKey/XTest work as on the host) and
  `--talk-name=org.kde.StatusNotifierWatcher`. Nothing else; keep it that way.
- Inside the sandbox (`FLATPAK_ID` set, `autostart::in_flatpak`): the instance socket is `$XDG_RUNTIME_DIR/app/$FLATPAK_ID/fastpaste.sock` (the only
  dir shared between `flatpak run`s); the tray is built with `disable_dbus_name(true)` (ksni's pid-based bus name may not be owned);
  Start at login goes through the Background portal (`autostart::portal`, blocks while the desktop asks, so `app.rs` calls it from a thread
  and resets the switch on a refusal); OCR runs `flatpak-spawn --host tesseract` (`ocr::tesseract`), which works only after the user's
  `flatpak override --talk-name=org.freedesktop.Flatpak`. Do not add that permission to the manifest.
- Auto-update = our own signed OSTree remote on GitHub Pages (`flatpak.yml`: `bundle` → `pages` → `release`), deployed on `v*` tags only.
  Every deploy replaces the whole site, so never deploy Pages from another trigger. The public key is `flatpak/signing-key.b64` (base64 of the
  binary export); the private half is the secret `FLATPAK_GPG_PRIVATE_KEY`. RELEASING.md lists the one-time setup.
- **Flathub policy:** manifests must NOT contain AI-generated content and AI must not open the submission PR. Do not write a Flathub manifest
  or submission text for the owner; `docs/FLATHUB.md` explains the process.
- The build vendors crates (`cargo vendor`), so `flatpak/vendor` must not be in the manifest's `skip:`. Keep build dirs out of the repo (Dropbox).

## Open items

- Not done: Wayland, image formats other than PNG and JPEG, a first real Flatpak build and run. See `TODO.md`.


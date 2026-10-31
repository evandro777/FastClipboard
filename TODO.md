# TODO

What is left to build, most useful first. Checked items are done and kept only for context.

## Next features
- [ ] Preferences UI for `hotkey`, `max_items`, `language` (today: edit `settings.toml`).

## Robustness
- [ ] History file: whole-file rewrite on every copy. Move to SQLite if it ever needs more than a few thousand items (see the `shortcut:` comment in `crates/core/src/history.rs`).
- [ ] Other image formats (JPEG, BMP, screenshots offered only as `image/jpeg`) are not recorded; only `image/png` is.
- [ ] Text and HTML over 1 MiB are skipped (INCR is only implemented for images).

## Packaging and release
- [ ] Real icon in `packaging/io.github.evandro777.FastPaste.svg` (the tray and autostart entry use the theme's `edit-paste` icon until then); rewrite the metainfo description and screenshots.
- [ ] Add a `LICENSE` file (GPL-3.0-or-later is declared in `Cargo.toml`).
- [ ] Check `appimage/build.sh` and the CI workflows after the rename (`ci.yml`, `appimage.yml`, `virustotal.yml`); `RELEASING.md` has the steps. The new `ksni`/`zbus` dependencies may need no extra system libs, but confirm in CI.
- [ ] Set the repository URL in `Cargo.toml` (currently derived from the template: `evandro777/FastPaste`).
- [ ] Translations: only `pt_BR` exists; add languages in `dev/gen_po.py`.

## Out of scope for now
- Wayland (no global shortcuts or clipboard snooping without compositor portals).
- Sync, encryption, tabs, scripting, plugins (the CopyQ features this app deliberately leaves out).
- A desktop notification when the hotkey is taken: that almost always means a second instance, which should just exit (the stderr message says so); another app holding the same key is fixed by changing `hotkey` in `settings.toml`.
- Pinning entries and Alt+number quick-pick (removed on purpose).

## Done
- [x] History store: dedupe, 500-item cap, search, atomic save, corrupt-file recovery.
- [x] Clipboard watcher (XFixes, text + HTML) and own-clipboard serving.
- [x] Global hotkey (default Ctrl+Alt+V), popup with recent (9) / all list and search.
- [x] Paste with formatting, plain-text paste, Ctrl+Shift+V for terminals.
- [x] Focus, close-on-focus-loss and paste sequencing ported from FastMoji, tested under Muffin.
- [x] Rename to FastPaste.
- [x] Manual tests on the real desktop: Shift+Enter plain paste, Firefox/Chrome HTML, terminal and GTK3 paste, Alt+D.
- [x] Removed Pin and Alt+1..9.
- [x] Autostart ("Start at login" in the tray menu, XDG autostart entry).
- [x] Skip password-manager copies (`x-kde-passwordManagerHint`).
- [x] Tray icon (StatusNotifier) with Open / Start at login / Clear history / Quit.
- [x] Remember the popup size.
- [x] Place the popup centered on the mouse pointer, inside its monitor, without a visible jump.
- [x] Collapse an HTML flavor identical to its plain text.
- [x] Images: PNG files + thumbnails, INCR receive for big ones, thumbnail rows, paste serves the original PNG.
- [x] Verified on the real desktop: tray, start at login, two-monitor placement, password-manager skip, idle memory, images from real apps, text-over-image choice, the old Shift+Enter paste.
- [x] Orphan files in `images/` are swept at startup (not when `history.toml` is missing or corrupt, so a restored history still finds its images).
- [x] Single instance via a Unix socket (`instance.rs`, ported from FastMoji): a second start opens the running popup, `fastpaste --toggle` flips it (bind it to a key where the global grab is not possible). Needs manual testing.
- [x] Shift+Enter opens an options menu (original, plain, trim, upper, lower, one line; replaces Ctrl+Enter and Alt+letter); paste releases held modifiers.

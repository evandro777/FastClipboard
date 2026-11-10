# Build

## How it is built

- **Rust + Slint** with the winit backend and the **software renderer** (no GPU/GL context, which is where most of the memory of a toolkit app goes).
- **X11 through `x11rb`**: one thread with the XFixes clipboard watcher, the `XGrabKey` global shortcut and the clipboard owner that serves a pick; XTEST Ctrl+V paste (Ctrl+Shift+V in terminals) and EWMH focus hand-over.
- **History** in `crates/core` (no UI code): deduplicated, capped, searched in memory, saved as one TOML file. Images are PNG files with a small thumbnail.
- **Tray** as a StatusNotifierItem (`ksni`, without tokio), optional: the app runs without one.
- **No async runtime.** Work runs in threads and is handed to the UI thread with `slint::invoke_from_event_loop`.
- **Single instance** through a Unix socket; a second start sends `show` (or `toggle`) to the first and exits.

More detail for contributors is in [`AGENTS.md`](AGENTS.md).

## First build

Install a Rust toolchain and the libraries it links.

Debian/Ubuntu/Mint:

```bash
sudo apt install libfontconfig-dev libxkbcommon-dev libxkbcommon-x11-dev libx11-dev libxcb1-dev rustup
rustup default stable
```

## How to run

```bash
cargo run --profile fast         # quick iteration build (no LTO), starts the daemon
cargo build --release            # thin LTO
cargo test
```

| Command              | Effect                                                         |
|----------------------|----------------------------------------------------------------|
| `fastclipboard`          | Open the popup (starts the daemon if needed)                   |
| `fastclipboard --toggle` | Open it, or close it if it is open (for your own shortcut)     |
| `fastclipboard --prefs`  | Open the preferences                                           |

## From a release (Flatpak bundle)

Download `FastClipboard-<version>-x86_64.flatpak` from the [Releases](https://github.com/evandro777/FastClipboard/releases) page, then:

```bash
flatpak install --user ./FastClipboard-<version>-x86_64.flatpak
flatpak run io.github.evandro777.FastClipboard
```

For automatic updates use the Flatpak remote described in the [README](README.md#install) instead.

## Build the Flatpak yourself

App id: `io.github.evandro777.FastClipboard`.

```bash
sudo apt install flatpak-builder
flatpak install --user flathub org.freedesktop.Sdk//25.08 org.freedesktop.Sdk.Extension.rust-stable//25.08
flatpak/build.sh --devel      # installs io.github.evandro777.FastClipboard.Devel next to a release
flatpak run io.github.evandro777.FastClipboard.Devel
```

`flatpak/build.sh` without `--devel` builds the real id; `--bundle` also writes `flatpak/fastclipboard.flatpak`. Crates are vendored into `flatpak/vendor` (git-ignored) because the sandboxed build has no network, and the build tree goes to `~/.cache/fastclipboard-flatpak` (set `FASTCLIPBOARD_FLATPAK_CACHE` to move it), so it stays out of the repository. The sandbox only gets the X11 socket, IPC and the tray's D-Bus name. Settings live in `~/.var/app/io.github.evandro777.FastClipboard/data/FastClipboard/`.

How the signed remote is built on GitHub: [`docs/FLATPAK-SIGNING.md`](docs/FLATPAK-SIGNING.md).

## Build the AppImage yourself

```bash
appimage/build.sh             # needs appimagetool on PATH; writes FastClipboard-<version>-x86_64.AppImage (+ .zsync)
```

## Settings and data

**☰ / Tray → Preferences…** (or `fastclipboard --prefs`) edits the appearance (follow the system, light or dark), the shortcut (click it and press the new keys), the number of entries kept and the language, and applies each change at once, without a restart (the only button is Close). A shortcut that another program already holds is refused and the old one stays. The popup stays open while Preferences or About is open, even if you click another window. It also has **Start at login** (writes `~/.config/autostart/fastclipboard.desktop`; in the Flatpak it asks the desktop through the Background portal, or writes `~/.config/autostart/io.github.evandro777.FastClipboard.desktop` itself when the optional `--filesystem=xdg-config/autostart` override is set), **Clear history** (asks first) and **About**. The same values live in `$XDG_DATA_HOME/FastClipboard/settings.toml` (defaults to `~/.local/share/FastClipboard`):

- `hotkey`: modifiers (`ctrl`, `alt`, `shift`, `super`) plus one letter or digit, default `"ctrl+alt+v"`.
- `max_items`: how many entries are kept, default `500` (the window accepts 10 to 5000).
- `language`: `""` follows the system locale; otherwise a code such as `"pt_BR"` or `"en"`.
- `theme`: `""` follows the system's light or dark mode; otherwise `"light"` or `"dark"`.
- `ocr_lang`: tesseract languages for "Text from image (OCR)", joined with `+`. Default `""`: see below.
- `width`, `height` (the size is saved when the popup closes).

History is in `history.toml` next to `settings.toml`; images are files in `images/` (the PNG and a tiny thumbnail), deleted with their entry. Text over 1 MiB and images over 8 MB (PNG only) are not stored, and a copy skipped for size is reported on stderr. Together, the text of all entries is capped at 4 MiB and the image files at 1 GiB: beyond that the oldest entries are dropped, even before `max_items` is reached. Everything copied is stored in clear text (except password-manager copies): use **Clear history** in the preferences or Alt+D when that matters.

Only one instance runs. Starting `fastclipboard` again opens the popup of the running one.

### Text from an image (OCR)

On an image entry, the Shift+Enter (or right click) menu is **Original** plus the **OCR ·** entries (plain text, trim, upper case, lower case, one line), which paste the text found in the picture (it is not added to the history). Text entries have no OCR entries. It runs the `tesseract` command, which is optional and not bundled:

Install `tesseract` itself, it is recommended that always install with English (`eng`, the fallback when the app's language has no pack), then one extra package per language you read (Debian/Ubuntu/Mint names):

```bash
sudo apt install tesseract-ocr tesseract-ocr-eng
```

| Language   | Code  | Extra command                         |
|------------|-------|---------------------------------------|
| English    | `eng` | `sudo apt install tesseract-ocr-eng`  |
| Português  | `por` | `sudo apt install tesseract-ocr-por`  |
| Español    | `spa` | `sudo apt install tesseract-ocr-spa`  |
| Français   | `fra` | `sudo apt install tesseract-ocr-fra`  |
| Deutsch    | `deu` | `sudo apt install tesseract-ocr-deu`  |
| Italiano   | `ita` | `sudo apt install tesseract-ocr-ita`  |

Other languages follow the same pattern (`tesseract-ocr-<code>`); other distributions name the packages alike (for example `tesseract-langpack-<code>` on Fedora).

To check the installed languages, run `tesseract --list-langs`.

By default the languages follow the **Language** of the app: the app's language first (`pt_BR` → `por`, also `es`, `fr`, `de`, `it`), then English, and only the packs that are installed, so a missing pack never breaks it. For anything else set `ocr_lang` to the installed codes joined with `+`, for example `ocr_lang = "eng+por"`; it overrides the automatic choice. Tesseract tries the languages together on the same image. Suggestion: always include `eng` and add the one other language you read most, with the main one first. More languages make it slower and a little less accurate. FastClipboard checks `tesseract` and the language packs it would use at start, after saving the preferences, and each time the popup opens while something is missing. When OCR cannot run, the **OCR ·** entries are dimmed; picking one opens the preferences, which say what is wrong (Flatpak without the permission below, `tesseract` not installed, or language packs missing, including those named in `ocr_lang`) and link to this section (only the Flatpak permission is shown as a command here). A working setup is not re-checked on every open, so if `tesseract` is removed later, the failure only appears on stderr (run `FASTCLIPBOARD_DEBUG=1 fastclipboard` from a terminal) and nothing is pasted. The popup closes at once; the text arrives a second or two later.

### Using your desktop's own shortcut (like CopyQ)

Besides the built-in global shortcut, you can bind a key in your desktop to a command, the way it is usually done for CopyQ. The commands talk to the running instance (table above).

On Cinnamon: **System Settings → Keyboard → Shortcuts → Custom Shortcuts → Add custom shortcut**, command `fastclipboard --toggle` (use the full path, such as `/home/you/.local/bin/fastclipboard`, or the AppImage path, if it is not on your `PATH`; for the Flatpak use `flatpak run io.github.evandro777.FastClipboard --toggle`), then click the shortcut column and press **Super+V**. FastClipboard must already be running (start it at login in the preferences); if it is not, the command starts it and the popup does not open until you run it again.

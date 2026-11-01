# FastPaste

A tiny clipboard history for Linux (X11), inspired by CopyQ. Rust + Slint, software renderer, no GL. It runs in the
background, remembers what you copy (text and HTML) and brings it back with a global shortcut.

Landing page: <https://evandro777.github.io/FastPaste/>

## Install

### Flatpak: easy install with automatic updates

Every release is also published as a Flatpak remote, so `flatpak update` (or your software centre) keeps FastPaste current.
A `.flatpak` file from Releases follows the same remote, so it updates too.

```bash
flatpak remote-add --system --if-not-exists fastpaste https://evandro777.github.io/FastPaste/fastpaste.flatpakrepo
flatpak install --system fastpaste io.github.evandro777.FastPaste
```

**`--system` vs `--user` matters for updates.** Linux Mint's Update Manager only looks at *system-wide* Flatpaks; a `--user`
install never appears there (update it by hand with `flatpak update --user`). The remote is GPG-signed, but the key lives in
this repository, so it is only as trustworthy as the repository. The runtime comes from Flathub
(`flatpak remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo`, with `--system` or `--user`).

For a desktop shortcut use the command `flatpak run io.github.evandro777.FastPaste --toggle`.

- **Start at login** asks the desktop through the Background portal; if you refuse, reset it with
  `flatpak permission-reset io.github.evandro777.FastPaste`.
- **OCR in Flatpak:** the sandbox cannot see the host's `tesseract`. It is off unless you grant it, on your own account:

  ```bash
  flatpak override --user --talk-name=org.freedesktop.Flatpak io.github.evandro777.FastPaste
  ```

  FastPaste then runs the host's `tesseract` through `flatpak-spawn --host`. This lets it start commands outside the sandbox,
  which removes most of what the sandbox protects, so it is your choice. The AppImage does not need it.
- The Flatpak keeps its data in `~/.var/app/io.github.evandro777.FastPaste/`, separate from the AppImage's. Run one of the
  two, not both: the second one cannot take the shortcut.

### AppImage (with delta updates)

Download `FastPaste-<version>-x86_64.AppImage` from Releases, `chmod +x` it and run it. It uses the fontconfig, xkbcommon and
X11 libraries already on your system. It carries update information, so AppImageUpdate, Gear Lever or AppManager download only
the changed parts; it does not update itself.

### From source

```bash
cargo run --profile fast
```

## Using it

Run it once and press **Ctrl+Alt+V**. The tray icon menu has **Open**, **Preferences…** and **Quit**; the **☰** button
next to the search box opens the preferences too.

| Key         | Action                                                      |
|-------------|-------------------------------------------------------------|
| type        | search (all words must match, case-insensitive)             |
| Up / Down   | move the selection                                          |
| Enter       | paste the selection, keeping its formatting                 |
| Shift+Enter | options menu (Up/Down, Enter, Esc)                          |
| PgUp / PgDn | move one page of rows at a time                            |
| ↓ / scroll to the end | the list shows 20 entries and loads the next 20 on reaching the end |
| Right click | the Shift+Enter options menu for the row under the pointer |
| Alt+D       | delete the entry                                            |
| Esc         | close                                                       |

Text, HTML and **images** (PNG or JPEG, stored as PNG, up to 8 MB) are recorded. An image shows as a thumbnail with its size; Enter pastes
the original PNG. Plain text and the other menu transforms paste text only (they do not change the stored entry).

The popup opens centered on the mouse pointer (kept inside that monitor), remembers the size you give it, and closes when it
loses focus. Pasting returns to that window and sends Ctrl+V (Ctrl+Shift+V in terminals). If it cannot be
refocused, nothing is sent and the entry stays on the clipboard. Copies from password managers (the
`x-kde-passwordManagerHint` flag, set by KeePassXC, Bitwarden and others) are never recorded.
`FASTPASTE_DEBUG=1 fastpaste` logs what it decides.

The tray icon needs a StatusNotifier host (Cinnamon, KDE and GNOME with the AppIndicator extension have one). Without
one the app prints a message and keeps working, only without the icon.

## Settings and data

**☰ / Tray → Preferences…** (or `fastpaste --prefs`) edits the shortcut (click it and press the new keys), the number of
entries kept and the language, and applies them on Save without a restart. A shortcut that another program already
holds is refused. It also has **Start at login** (writes `~/.config/autostart/fastpaste.desktop`), **Clear history**
(asks first) and **About**. The same values live in `$XDG_DATA_HOME/FastPaste/settings.toml` (defaults to `~/.local/share/FastPaste`):

- `hotkey`: modifiers (`ctrl`, `alt`, `shift`, `super`) plus one letter or digit, default `"ctrl+alt+v"`.
- `max_items`: how many entries are kept, default `500` (the window accepts 10 to 5000).
- `language`: `""` follows the system locale; otherwise a code such as `"pt_BR"` or `"en"`.
- `ocr_lang`: tesseract languages for "Text from image (OCR)", joined with `+`. Default `""`: see below.
- `width`, `height` (the size is saved when the popup closes).

### Text from an image (OCR)

On an image entry, the Shift+Enter (or right click) menu is **Original** plus the **OCR ·** entries (plain text, trim,
upper case, lower case, one line), which paste the text found in the picture (it is not added to the history). Text
entries have no OCR entries. It runs the `tesseract` command, which is optional and not bundled:

```bash
sudo apt install tesseract-ocr tesseract-ocr-eng tesseract-ocr-por   # one tesseract-ocr-<code> package per language
tesseract --list-langs                                               # what is installed
```

By default the languages follow the **Language** of the app: the app's language first (`pt_BR` → `por`, also `es`,
`fr`, `de`, `it`), then English, and only the packs that are installed, so a missing pack never breaks it. For anything else
set `ocr_lang` to the installed codes joined with `+`, for example `ocr_lang = "eng+por"`; it overrides the automatic
choice. Tesseract tries the languages together on the same image. Suggestion: always include `eng` and add the one other
language you read most, with the main one first. More languages make it slower and a little less accurate. A missing language
pack or a missing `tesseract` is reported on stderr (run `FASTPASTE_DEBUG=1 fastpaste` from a terminal) and nothing is
pasted. The popup closes at once; the text arrives a second or two later.

History is in `history.toml` next to `settings.toml`; images are files in `images/` (the PNG and a tiny thumbnail), deleted with
their entry. Text over 1 MiB and images over 8 MB are not stored. Everything copied is stored in clear text
(except password-manager copies): use **Clear history** in the preferences or Alt+D when that matters.

Only one instance runs. Starting `fastpaste` again opens the popup of the running one.

### Using your desktop's own shortcut (like CopyQ)

Besides the built-in global shortcut, you can bind a key in your desktop to a command, the way it is usually done for
CopyQ. The commands talk to the running instance:

| Command             | Action                                       |
|---------------------|----------------------------------------------|
| `fastpaste --toggle` | opens the popup, or closes it if it is open |
| `fastpaste`          | opens the popup                             |
| `fastpaste --prefs`  | opens the preferences                       |

On Cinnamon: **System Settings → Keyboard → Shortcuts → Custom Shortcuts → Add custom shortcut**, command
`fastpaste --toggle` (use the full path, such as `/home/you/.local/bin/fastpaste`, or the AppImage path, if it is not
on your `PATH`), then click the shortcut column and press **Super+V**. FastPaste must already be running (start it at
login from the tray menu); if it is not, the command starts it and the popup does not open until you run it again.
The built-in shortcut stays active too: pick one that does not clash, or if you only want the desktop one, set the
built-in `hotkey` to something you never press. Super+V may already be taken by the desktop; remove that binding first.

Not supported: Wayland, files, image formats other than PNG and JPEG (a JPEG is stored as PNG).

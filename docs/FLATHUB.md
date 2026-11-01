# Publishing on Flathub

Notes for the maintainer. Flathub reviews each new app, and its rules matter here:

## Rules that affect this project (check docs.flathub.org/docs/for-app-authors/requirements; they change)

- **App id.** Projects hosted on GitHub must use `io.github.<user>.<App>`. This project uses `io.github.evandro777.FastPaste`,
  and the id must equal the `<id>` in the metainfo.
- **AI policy.** Submitters must disclose AI-generated code, documentation or packaging. **Flathub manifests must not contain
  AI-generated or AI-assisted content**, and AI tools or agents must not open or automate the submission pull request. This
  project's code, docs and the in-repo `flatpak/` files were written with an AI assistant, so:
    - **Write the Flathub manifest yourself**, by hand (you may read `flatpak/io.github.evandro777.FastPaste.yml` to
      understand it, but do not paste it).
    - **Open the submission PR yourself**, and disclose the AI assistance for the code and docs in it.
- **No network during the build.** Dependencies must be declared as sources. For Rust that means a `cargo-sources.json` from
  [`flatpak-cargo-generator.py`](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo) and `Cargo.lock`,
  referenced from the manifest (the in-repo manifest vendors crates with `cargo vendor` instead, which Flathub will not take).
- **Permissions as small as possible.** FastPaste needs the X11 socket (clipboard watcher, global key, paste), IPC and
  the StatusNotifier watcher name. Expect questions about `--socket=x11`: there is no Wayland alternative for a global
  shortcut and paste. OCR through `flatpak-spawn --host` is an opt-in override the user grants; it is not in the manifest.
- **Metainfo must validate** (`appstreamcli validate`), with screenshots served over HTTPS (for example
  `https://raw.githubusercontent.com/evandro777/FastPaste/master/docs/screenshots/popup.png`), a license, release notes and an
  OARS content rating. The icon is the SVG in `packaging/`.
- The submission goes to [flathub/flathub](https://github.com/flathub/flathub) as a pull request against the `new-pr` branch;
  after approval the app gets its own `flathub/io.github.evandro777.FastPaste` repo, and updates are PRs there.

## Checklist before submitting

- [ ] Repository is public with a release tag and the metainfo URLs resolve (homepage, bugtracker, screenshots).
- [ ] The Flatpak builds and runs from a clean machine (`flatpak/build.sh --devel`), including tray, hotkey, paste, the
      single-instance socket and Start at login (Background portal) inside the sandbox. **Not yet verified.**
- [ ] `cargo-sources.json` generated and builds offline.
- [ ] Hand-written Flathub manifest, with the AI disclosure in the PR text.
- [ ] Decide what to say about Wayland: it is X11 only.

## Until then

The signed Flatpak remote on GitHub Pages (`flatpak.yml`) is the distribution channel, with the bundle on GitHub Releases.

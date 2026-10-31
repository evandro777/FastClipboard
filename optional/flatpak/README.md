# Flatpak (optional)

Not wired in. Skip it for apps that need host access (`systemctl`, `/proc`, root), since the sandbox blocks it.

To add it, copy from FastMoji: `flatpak/build.sh`, `flatpak/*.yml` (manifest), `cargo-config.toml`, the GPG signing setup and
`.github/workflows/flatpak.yml`, then rename the app id and point the manifest at `packaging/`. FastMoji's own Flatpak
build is still unverified (see its AGENTS.md), so verify it there first. Do not copy `signing-key.b64`; generate a new key.

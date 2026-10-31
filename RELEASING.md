# Releasing

Releases are Git tags. CI builds the AppImage (+ `.zsync`) and attaches it to the GitHub release; VirusTotal then scans it.

1. Pick the version (`0.2.0`). Update **both**:
   - `Cargo.toml` → `[workspace.package] version`
   - `packaging/io.github.evandro777.FastPaste.metainfo.xml` → add a `<release version="0.2.0" date="YYYY-MM-DD">` entry (newest first)
2. `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
3. Merge to `master` through a pull request, wait for CI.
4. Tag and push (the tag must equal the version, or `appimage.yml` fails):

   ```bash
   git tag v0.2.0 -m "FastPaste 0.2.0"
   git push origin v0.2.0
   ```

5. Smoke-test the downloaded AppImage on a clean user.

One-time: add the repository secret `VIRUSTOTAL_API_KEY` (or delete `virustotal.yml`).

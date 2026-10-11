# Changelog

## 0.2.0 — 2026-10-10

### Added
- `install.sh` installs a desktop entry (`Name=Cahier`, app id `com.clouddown.cahier`) and a scalable icon, and `--uninstall` removes them.
- `install.sh` downloads the latest Linux x86_64 release binary. `--from-source` still compiles `main`.
- `fc-match` (fontconfig) is checked, and installed on Arch when it is missing.
- GitHub Actions runs `cargo fmt`, `cargo clippy -D warnings`, and `cargo test`. Tags `v*` publish a Linux x86_64 binary on the GitHub Release.
- A failed save shows a toast instead of disappearing.
- README documents an optional Hyprland keybinding and window rule.

### Changed
- The window title is Cahier.
- Notebook and shelf saves are atomic: write a temp file in the same directory, fsync, then rename.
- Version is 0.2.0. The crate now requires Rust 1.88, which the locked dependencies already need.

### Fixed
- Empty strokes no longer panic while the ribbon is built or a shape is snapped.
- Small-window layout, with a minimum size of 320×280 (2026-10-03).
- One Download… menu with a PNG/PDF picker (2026-10-03).
- README is in English (2026-10-03).

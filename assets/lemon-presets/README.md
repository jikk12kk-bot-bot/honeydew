# Lemon plugin presets

Built into the app (`build.rs`, `src/plugins/presets.rs`) and listed by each Lemon plugin's preset button.

- `<plugin>/[<category>/]<name>.ospreset` outside the `Honeydew*` folders: Lemon Studio's factory presets
  (`crates/os-app/factory-presets`, GPL-3.0), unchanged.
- `Honeydew*` folders: Honeydew's own presets, written by `tools/lemon_presets.py` (run it from the repository root
  after editing it; it rewrites those folders only).
- Citrus has no files here: its presets are made in code from its own state type (`src/plugins/citrus_presets.rs`)
  and list as `Presets/Citrus/…` all the same.

The format: Lemon Studio's `factory-presets/README.md` (summarised in `src/plugins/presets.rs`).

## [1.0.2] - 2026-11-19

### Added
- **Theme setting** in Settings: Auto (follow Windows), Dark or Light. The choice is saved between runs and applies immediately, without pressing Apply.
- **Light theme colors.** The accent, status, error and button colors now have light-mode versions with higher contrast. Dark mode looks the same as before.
- **App icon:** a sand-gold "S" drawn in the style of the Dune film title lettering. It is used for the window, taskbar and Alt+Tab, and is embedded in the `.exe` for Explorer.
- **Automatic settings migration.** On first launch, settings from older versions (`config.json`, `window.json`) are imported into `settings.toml` without any prompt. Migration works from v1.0.0.

### Changed
- All settings (save location, Steam profile, vault, screenshot folders, SQL overrides, game log path, theme, window size and position) now live in a single `settings.toml` in the config folder (`%APPDATA%\DuneSaveManager`), shared by the GUI and the CLI. Each part is written separately, so the GUI, the window tracker and the CLI no longer overwrite each other's values.
- The app now follows the Windows light/dark setting live, including the title bar, instead of relying on defaults.
- The theme is applied before the first frame, so there is no dark flash on startup when Light is selected.
- The success message in the status bar now uses the same green as the "Game closed" indicator.
- `config reset` in the CLI now resets only the settings and keeps the theme and window size.
- The build now includes a `build.rs` step that embeds the icon (new Windows-only build dependency: `winresource`). The `toml` crate is a new dependency.

### Notes
- Migrated files are renamed to `*.json.migrated` and kept as a backup. A legacy file that cannot be parsed is left untouched.
- Downgrading is not supported: older versions do not read `settings.toml`. To go back, rename the `*.json.migrated` files to `.json`.
- If embedding the exe icon fails, the build prints a warning and continues. Only the file icon in Explorer is missing.
- If you force a theme that differs from Windows and then change the Windows theme, the title bar may briefly flicker before it is corrected.


# Footnotes
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

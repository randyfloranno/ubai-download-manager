# UDM documentation

UDM (Ubai Download Manager) is a desktop download manager that splits large files into parallel connections and resumes safely after a pause, a dropped connection or a restart.

- [Known gaps](gaps.md)
- [Feature parity checklist](parity-checklist-download-manager.md)
- [Command-line client](cli.md)
- [Browser integration](browser-integration.md)

## Using the main window

- **Right-click a download:** *Open* (finished downloads), *Open file location* (shows the file selected in the file manager), resume/pause, restart, verify, remove, properties.
- **Download complete dialog:** shows where the file was saved, with *Open*, *Open folder* and *Close*. Turn it off with *Don't show this dialog again* or in Settings.
- **Keyboard:** <kbd>Enter</kbd> opens the selected finished download (properties for unfinished ones), <kbd>Delete</kbd> asks to remove it, <kbd>Ctrl</kbd>+<kbd>V</kbd> opens *Add download* with the copied link.
- **Default download folder:** the Downloads folder your desktop uses (Windows known folder, XDG user dirs on Linux), else `<home>/Downloads`.

## Building a release folder

```powershell
cargo xtask dist
```

This builds `udm`, `udm-cli` and `udm-native-host` in release mode without the developer `--sample` feature, builds the browser extension, and collects everything with the docs and third-party licences in `target/dist/udm-<version>-<os>-<arch>/`. Installers and code signing are not provided yet.

## Licence

UDM is dual-licensed under the MIT licence (`LICENSE-MIT`) or the Apache License 2.0 (`LICENSE-APACHE`), at your option. Third-party assets are listed in `ATTRIBUTION.md`; third-party crate licences are collected by `cargo xtask licences`.

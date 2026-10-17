# UDM documentation

UDM (Ubai Download Manager) is a desktop download manager that splits large files into parallel connections and resumes safely after a pause, a dropped connection or a restart.

- [Known gaps](gaps.md)
- [Feature parity checklist](parity-checklist-download-manager.md)
- [Command-line client](cli.md)
- [Browser integration](browser-integration.md)

## Building a release folder

```powershell
cargo xtask dist
```

This builds `udm`, `udm-cli` and `udm-native-host` in release mode without the developer `--sample` feature, builds the browser extension, and collects everything with the docs and third-party licences in `target/dist/udm-<version>-<os>-<arch>/`. Installers and code signing are not provided yet.

# Known gaps and deliberate non-features

## Won't do
- No DRM handling, no decryption of encrypted streams, no paywall, captcha or login-wall bypass, no stream ripping.

## Not yet done
- Tray icon on macOS and Linux: Windows has one; the Linux tray backend pulls in an MPL-2.0 crate, and macOS is not tested yet.
- Installer, code signing and notarisation: `cargo xtask dist` builds a portable folder; Windows may warn about an unknown publisher.
- Store extension ids for Chrome and Edge: assigned on publication; until then register the unpacked extension with `udm-cli native-host install --extension-id <id>`.
- Auto-update: none; install a newer build over the old one.
- Default download folder is `<home>/Downloads` (`%USERPROFILE%\Downloads` on Windows); a Downloads folder moved elsewhere by the OS (known-folder redirection, XDG user dirs) is not detected yet. Change it in Settings.

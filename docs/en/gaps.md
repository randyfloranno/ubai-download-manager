# Known gaps and deliberate non-features

## Won't do
- No DRM handling, no decryption of encrypted streams, no paywall, captcha or login-wall bypass, no stream ripping.

## Not yet done
- Tray icon (waits on a dependency decision).
- Installer, code signing and notarisation: `cargo xtask dist` builds a portable folder; Windows may warn about an unknown publisher.
- Store extension ids for Chrome and Edge: assigned on publication; until then register the unpacked extension with `udm-cli native-host install --extension-id <id>`.
- Auto-update: none; install a newer build over the old one.
- Application logo/icon.

# Browser integration

The UDM extension (Chrome, Edge, Firefox) hands downloads and links to UDM through the native messaging host `udm-native-host`.

## What it captures

- **Downloads the browser starts**, when interception is on, the file extension is in the list (archives, programs, disk images, video, audio, documents) and the size is unknown or at least 1 MiB.
- **Right-click → "Download link with UDM"** on a link, and **"Download this media with UDM"** on a video, audio or image element whose source is a direct file URL.
- Only `http`, `https` and `ftp` URLs without embedded credentials. `blob:` and `data:` URLs, encrypted streams and DRM-protected media are never captured.

Cookies are sent only for the captured URL, read from the browser at the moment of capture, and stored by UDM in the OS keychain. UDM never reads browser profile files.

## Build and load

```powershell
cargo xtask ext        # writes extension/dist/chrome and extension/dist/firefox
```

- Chrome/Edge: `chrome://extensions` → Developer mode → *Load unpacked* → `extension/dist/chrome`. Note the extension id.
- Firefox: `about:debugging` → *Load Temporary Add-on* → `extension/dist/firefox/manifest.json` (id `udm@udm.dev`).

## Register the native host

Run once, after loading the extension (use the id shown by `chrome://extensions`; repeat `--extension-id` for Edge if it differs):

```powershell
udm-cli native-host install --extension-id <extension-id>
udm-cli native-host uninstall
```

This writes `dev.udm.native_host.json` and `dev.udm.native_host-firefox.json` into `<data-dir>/native-host/` and registers them for the current user (Windows: the Chrome, Edge and Firefox `NativeMessagingHosts` keys under `HKCU`; macOS/Linux: each browser's per-user `NativeMessagingHosts` folder). `--host <path>` overrides the host binary (default: next to `udm-cli`).

### Manual registration

Create `dev.udm.native_host.json` next to `udm-native-host.exe`:

```json
{
  "name": "dev.udm.native_host",
  "description": "UDM",
  "path": "C:\\Program Files\\UDM\\udm-native-host.exe",
  "type": "stdio",
  "allowed_origins": ["chrome-extension://<extension-id>/"]
}
```

For Firefox, write a second file `dev.udm.native_host-firefox.json` with `"allowed_extensions": ["udm@udm.dev"]` instead of `allowed_origins`. Then register them (Windows):

```powershell
reg add "HKCU\Software\Google\Chrome\NativeMessagingHosts\dev.udm.native_host" /ve /t REG_SZ /d "C:\Program Files\UDM\dev.udm.native_host.json" /f
reg add "HKCU\Software\Microsoft\Edge\NativeMessagingHosts\dev.udm.native_host" /ve /t REG_SZ /d "C:\Program Files\UDM\dev.udm.native_host.json" /f
reg add "HKCU\Software\Mozilla\NativeMessagingHosts\dev.udm.native_host" /ve /t REG_SZ /d "C:\Program Files\UDM\dev.udm.native_host-firefox.json" /f
```

## How it works

The host reads one message and checks it: size ≤ 1 MiB, URL scheme and length, and the size of each field. It then forwards the message to the running app on a local, token-protected endpoint described in `ipc.json` in the data folder. If UDM is not running, the toolbar button shows `!` and the reason.

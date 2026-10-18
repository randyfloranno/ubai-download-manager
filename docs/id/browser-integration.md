# Integrasi peramban

Ekstensi UDM (Chrome, Edge, Firefox) menyerahkan unduhan dan tautan ke UDM melalui host native messaging `udm-native-host`.

## Apa yang ditangkap

- **Unduhan yang dimulai peramban**, bila penangkapan aktif, ekstensi berkas ada dalam daftar (arsip, program, citra cakram, video, audio, dokumen) dan ukurannya tidak diketahui atau minimal 1 MiB.
- **Klik kanan → "Unduh tautan dengan UDM"** pada tautan, dan **"Unduh media ini dengan UDM"** pada elemen video, audio, atau gambar yang sumbernya URL berkas langsung.
- **Tombol "Unduh dengan UDM"** di atas `<video>`/`<audio>` yang sedang diputar bila sumbernya berkas media langsung (mp4, webm, mkv, mp3, m4a, flac, …). Tombol ini tidak pernah muncul di platform streaming (YouTube, Netflix, Twitch, Vimeo, Spotify, …), untuk sumber `blob:` (aliran adaptif), atau untuk manifes aliran (`.m3u8`, `.mpd`). Matikan dengan `"media_button": false` di pengaturan ekstensi.
- Hanya URL `http`, `https`, dan `ftp` tanpa kredensial di dalamnya. URL `blob:` dan `data:`, stream terenkripsi, dan media ber-DRM tidak pernah ditangkap.

Cookie hanya dikirim untuk URL yang ditangkap, dibaca dari peramban saat penangkapan, dan disimpan UDM di keychain sistem operasi. UDM tidak pernah membaca berkas profil peramban.

## Membangun dan memuat

```powershell
cargo xtask ext        # menulis extension/dist/chrome dan extension/dist/firefox
```

- Chrome/Edge: `chrome://extensions` → Mode pengembang → *Load unpacked* → `extension/dist/chrome`. Catat id ekstensi.
- Firefox: `about:debugging` → *Load Temporary Add-on* → `extension/dist/firefox/manifest.json` (id `udm@randyfloranno.github.io`).

## Mendaftarkan host native

Jalankan sekali setelah ekstensi dimuat (pakai id yang tampil di `chrome://extensions`; ulangi `--extension-id` untuk Edge bila berbeda):

```powershell
udm-cli native-host install --extension-id <id-ekstensi>
udm-cli native-host uninstall
```

Perintah ini menulis `io.github.randyfloranno.udm.json` dan `io.github.randyfloranno.udm-firefox.json` ke `<folder-data>/native-host/` lalu mendaftarkannya untuk pengguna saat ini (Windows: kunci `NativeMessagingHosts` milik Chrome, Edge, dan Firefox di `HKCU`; macOS/Linux: folder `NativeMessagingHosts` per pengguna milik tiap peramban). `--host <path>` mengganti lokasi biner host (bawaan: di samping `udm-cli`).

### Pendaftaran manual

Buat `io.github.randyfloranno.udm.json` di samping `udm-native-host.exe`:

```json
{
  "name": "io.github.randyfloranno.udm",
  "description": "UDM",
  "path": "C:\\Program Files\\UDM\\udm-native-host.exe",
  "type": "stdio",
  "allowed_origins": ["chrome-extension://<id-ekstensi>/"]
}
```

Untuk Firefox, tulis berkas kedua `io.github.randyfloranno.udm-firefox.json` dengan `"allowed_extensions": ["udm@randyfloranno.github.io"]` sebagai pengganti `allowed_origins`. Lalu daftarkan (Windows):

```powershell
reg add "HKCU\Software\Google\Chrome\NativeMessagingHosts\io.github.randyfloranno.udm" /ve /t REG_SZ /d "C:\Program Files\UDM\io.github.randyfloranno.udm.json" /f
reg add "HKCU\Software\Microsoft\Edge\NativeMessagingHosts\io.github.randyfloranno.udm" /ve /t REG_SZ /d "C:\Program Files\UDM\io.github.randyfloranno.udm.json" /f
reg add "HKCU\Software\Mozilla\NativeMessagingHosts\io.github.randyfloranno.udm" /ve /t REG_SZ /d "C:\Program Files\UDM\io.github.randyfloranno.udm-firefox.json" /f
```

## Cara kerja

Host membaca satu pesan lalu memeriksanya: ukuran ≤ 1 MiB, skema dan panjang URL, serta ukuran tiap kolom. Setelah itu pesan diteruskan ke aplikasi yang sedang berjalan melalui endpoint lokal bertoken yang dijelaskan di `ipc.json` pada folder data. Jika UDM tidak berjalan, tombol toolbar menampilkan `!` beserta alasannya.

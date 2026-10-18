# UDM — Ubai Download Manager

*[English](#english) · [Bahasa Indonesia](#bahasa-indonesia)*

## English

UDM is a desktop download manager written in Rust. It speeds up large downloads by splitting a file into several parallel connections and resumes safely after a pause, a dropped connection or a PC restart.

### Features

- Multi-segment downloads with dynamic segmentation over HTTP/1.1, HTTP/2, HTTPS, FTP and FTPS
- Safe resume: the remote file is re-validated (ETag, Last-Modified, size) and every byte range is verified before the file is finalised
- Queues, scheduler (start/stop time, shutdown or sleep when done), global and per-download speed limits
- Automatic categories (Documents, Video, Music, Programs, Archives) with target folders
- MD5 / SHA-256 checksum verification, history and search
- Browser integration for Chrome, Edge and Firefox (extension + native messaging), clipboard capture
- Batch download from a URL list, simple site grabber
- Proxy (HTTP CONNECT, SOCKS5) and Basic/Digest authentication; passwords are kept only in the OS keychain
- System tray icon on Windows (closing the window keeps downloads running)
- Indonesian and English user interface; no telemetry

UDM only downloads content that is directly accessible to you. It does not handle DRM, encrypted streams, paywalls or captchas.

### Getting started

Requires a stable Rust toolchain. On Windows (PowerShell):

```powershell
cargo xtask dist                 # release folder in target/dist/udm-<version>-<os>-<arch>/
cargo run -p udm-app             # run the desktop app from source
cargo run -p udm-cli -- add https://example.com/file.zip --wait
```

Documentation: [docs/en](docs/en/README.md) · Roadmap: [ROADMAP.md](ROADMAP.md) · Known gaps: [docs/en/gaps.md](docs/en/gaps.md)

### Contributing

Repository rules are in [AGENTS.md](AGENTS.md). `cargo xtask ci` must pass before every commit.

### Licence

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Third-party assets are listed in [ATTRIBUTION.md](ATTRIBUTION.md).

## Bahasa Indonesia

UDM adalah pengelola unduhan desktop yang ditulis dengan Rust. UDM mempercepat unduhan besar dengan membagi berkas ke beberapa koneksi paralel, dan melanjutkan unduhan dengan aman setelah dijeda, koneksi terputus, atau PC dimulai ulang.

### Fitur

- Unduhan multisegmen dengan segmentasi dinamis melalui HTTP/1.1, HTTP/2, HTTPS, FTP, dan FTPS
- Lanjutkan dengan aman: berkas di server diperiksa ulang (ETag, Last-Modified, ukuran) dan setiap rentang byte diverifikasi sebelum berkas difinalisasi
- Antrean, penjadwal (waktu mulai/berhenti, matikan atau tidurkan PC setelah selesai), pembatas kecepatan global dan per unduhan
- Kategori otomatis (Dokumen, Video, Musik, Program, Arsip) dengan folder tujuan
- Verifikasi checksum MD5 / SHA-256, riwayat dan pencarian
- Integrasi peramban untuk Chrome, Edge, dan Firefox (ekstensi + native messaging), tangkap papan klip
- Unduh massal dari daftar URL, penyedot situs sederhana
- Proksi (HTTP CONNECT, SOCKS5) dan autentikasi Basic/Digest; kata sandi hanya disimpan di keychain sistem operasi
- Ikon baki sistem di Windows (menutup jendela tidak menghentikan unduhan)
- Antarmuka bahasa Indonesia dan Inggris; tanpa telemetri

UDM hanya mengunduh konten yang dapat Anda akses langsung. UDM tidak menangani DRM, aliran terenkripsi, paywall, atau captcha.

### Memulai

Membutuhkan toolchain Rust stabil. Di Windows (PowerShell):

```powershell
cargo xtask dist                 # folder rilis di target/dist/udm-<versi>-<os>-<arsitektur>/
cargo run -p udm-app             # jalankan aplikasi desktop dari kode sumber
cargo run -p udm-cli -- add https://example.com/file.zip --wait
```

Dokumentasi: [docs/id](docs/id/README.md) · Peta jalan: [ROADMAP.md](ROADMAP.md) · Celah yang diketahui: [docs/id/gaps.md](docs/id/gaps.md)

### Berkontribusi

Aturan repositori ada di [AGENTS.md](AGENTS.md) (terjemahan: [AGENTS.id.md](AGENTS.id.md)). `cargo xtask ci` harus lulus sebelum setiap commit.

### Lisensi

Berlisensi ganda [MIT](LICENSE-MIT) atau [Apache-2.0](LICENSE-APACHE), sesuai pilihan Anda. Aset pihak ketiga tercantum di [ATTRIBUTION.md](ATTRIBUTION.md).

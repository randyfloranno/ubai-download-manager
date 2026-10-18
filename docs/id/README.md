# Dokumentasi UDM

UDM (Ubai Download Manager) adalah pengelola unduhan desktop yang membagi berkas besar ke beberapa koneksi paralel dan melanjutkan dengan aman setelah jeda, koneksi terputus, atau mulai ulang.

- [Celah yang diketahui](gaps.md)
- [Daftar periksa kesetaraan fitur](parity-checklist-download-manager.md)
- [Klien baris perintah](cli.md)
- [Integrasi peramban](browser-integration.md)

## Memakai jendela utama

- **Klik kanan sebuah unduhan:** *Buka* (unduhan yang sudah selesai), *Buka lokasi berkas* (menampilkan berkas terpilih di pengelola berkas), lanjutkan/jeda, ulangi, verifikasi, hapus, properti.
- **Dialog unduhan selesai:** menampilkan lokasi penyimpanan berkas, dengan tombol *Buka*, *Buka folder* dan *Tutup*. Matikan lewat *Jangan tampilkan dialog ini lagi* atau di Pengaturan.
- **Papan ketik:** <kbd>Enter</kbd> membuka unduhan terpilih yang sudah selesai (properti untuk yang belum selesai), <kbd>Delete</kbd> meminta konfirmasi hapus, <kbd>Ctrl</kbd>+<kbd>V</kbd> membuka *Tambah unduhan* berisi tautan yang disalin.
- **Folder unduhan bawaan:** folder Downloads yang dipakai desktop Anda (known folder Windows, XDG user dirs di Linux), atau `<home>/Downloads` bila tidak ada.

## Membuat folder rilis

```powershell
cargo xtask dist
```

Perintah ini membangun `udm`, `udm-cli`, dan `udm-native-host` dalam mode rilis tanpa fitur pengembang `--sample`, membangun ekstensi peramban, lalu mengumpulkan semuanya beserta dokumentasi dan lisensi pihak ketiga di `target/dist/udm-<versi>-<os>-<arsitektur>/`. Installer dan penandatanganan kode belum tersedia.

## Lisensi

UDM berlisensi ganda: lisensi MIT (`LICENSE-MIT`) atau Apache License 2.0 (`LICENSE-APACHE`), sesuai pilihan Anda. Aset pihak ketiga tercantum di `ATTRIBUTION.md`; lisensi crate pihak ketiga dikumpulkan oleh `cargo xtask licences`.

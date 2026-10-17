# Dokumentasi UDM

UDM (Ubai Download Manager) adalah pengelola unduhan desktop yang membagi berkas besar ke beberapa koneksi paralel dan melanjutkan dengan aman setelah jeda, koneksi terputus, atau mulai ulang.

- [Celah yang diketahui](gaps.md)
- [Daftar periksa kesetaraan fitur](parity-checklist-download-manager.md)
- [Klien baris perintah](cli.md)
- [Integrasi peramban](browser-integration.md)

## Membuat folder rilis

```powershell
cargo xtask dist
```

Perintah ini membangun `udm`, `udm-cli`, dan `udm-native-host` dalam mode rilis tanpa fitur pengembang `--sample`, membangun ekstensi peramban, lalu mengumpulkan semuanya beserta dokumentasi dan lisensi pihak ketiga di `target/dist/udm-<versi>-<os>-<arsitektur>/`. Installer dan penandatanganan kode belum tersedia.

## Lisensi

UDM berlisensi ganda: lisensi MIT (`LICENSE-MIT`) atau Apache License 2.0 (`LICENSE-APACHE`), sesuai pilihan Anda. Aset pihak ketiga tercantum di `ATTRIBUTION.md`; lisensi crate pihak ketiga dikumpulkan oleh `cargo xtask licences`.

# Celah yang diketahui dan bukan-fitur yang disengaja

## Tidak akan dibuat
- Tidak ada penanganan DRM, dekripsi aliran terenkripsi, penembusan paywall, captcha, atau dinding login, maupun perekaman aliran.

## Belum selesai
- Ikon baki sistem di macOS dan Linux: Windows sudah punya; backend baki Linux menarik crate berlisensi MPL-2.0, dan macOS belum diuji.
- Installer, penandatanganan kode, dan notarisasi: `cargo xtask dist` membuat folder portabel; Windows dapat memperingatkan penerbit tidak dikenal.
- ID ekstensi toko untuk Chrome dan Edge: diberikan saat publikasi; sampai saat itu daftarkan ekstensi unpacked dengan `udm-cli native-host install --extension-id <id>`.
- Pembaruan otomatis: belum ada; pasang build yang lebih baru di atas yang lama.
- Folder unduhan bawaan adalah folder Downloads milik desktop (known folder Windows, `user-dirs.dirs` XDG di Linux), atau `<home>/Downloads` bila tidak ada. Di Windows, folder Downloads yang dialihkan ke path berisi karakter non-ASCII belum terdeteksi dan kembali ke `%USERPROFILE%\Downloads`. Ubah di Pengaturan.

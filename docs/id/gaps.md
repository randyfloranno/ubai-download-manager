# Celah yang diketahui dan bukan-fitur yang disengaja

## Tidak akan dibuat
- Tidak ada penanganan DRM, dekripsi aliran terenkripsi, penembusan paywall, captcha, atau dinding login, maupun perekaman aliran.

## Belum selesai
- Ikon baki sistem (menunggu keputusan dependensi).
- Installer, penandatanganan kode, dan notarisasi: `cargo xtask dist` membuat folder portabel; Windows dapat memperingatkan penerbit tidak dikenal.
- ID ekstensi toko untuk Chrome dan Edge: diberikan saat publikasi; sampai saat itu daftarkan ekstensi unpacked dengan `udm-cli native-host install --extension-id <id>`.
- Pembaruan otomatis: belum ada; pasang build yang lebih baru di atas yang lama.
- Logo/ikon aplikasi.

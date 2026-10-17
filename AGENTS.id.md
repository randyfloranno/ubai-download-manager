# AGENTS.id.md — UDM (Ubai Download Manager)

> **Catatan:** berkas ini adalah terjemahan bahasa Indonesia dari `AGENTS.md`. `AGENTS.md` (bahasa Inggris) adalah versi kanonis. **Jika ada perbedaan, yang berlaku adalah `AGENTS.md`.**

---

## 1. Produk

**UDM (Ubai Download Manager)** adalah aplikasi desktop pengelola unduhan yang mempercepat dan melindungi unduhan besar dengan memecah berkas menjadi beberapa koneksi paralel (multi-segmen, segmentasi dinamis) dan melanjutkan unduhan dengan aman setelah dijeda, koneksi putus, atau PC dinyalakan ulang.

- **Setara fitur dengan:** Internet Download Manager (IDM). IDM hanya *acuan perilaku* (lihat §5.1, clean-room).
- **Pengguna:** pengguna umum, staf kantor, dan orang IT yang sering mengunduh berkas besar di koneksi tidak stabil.
- **Fitur inti:** unduhan multi-segmen dengan segmentasi dinamis; jeda/lanjutkan (juga setelah koneksi putus atau restart); antrean dan penjadwal (jam mulai, jam berhenti, matikan/tidurkan PC setelah selesai); batas kecepatan per unduhan dan global; kategori otomatis per jenis berkas (Dokumen, Video, Musik, Program, Arsip) dengan folder tujuan; integrasi browser (Chrome, Edge, Firefox) lewat ekstensi + native messaging untuk menangkap tautan dan media yang dapat diakses langsung; tangkap tautan dari clipboard; unduh massal dari daftar URL; site grabber sederhana; proxy dan autentikasi (Basic/Digest, cookie yang diserahkan browser); verifikasi checksum (MD5/SHA-256); riwayat dan pencarian; notifikasi selesai.
- **Protokol:** HTTP/1.1, HTTP/2, HTTPS, FTP, FTPS.
- **Platform:** Windows, macOS, Linux (desktop). WebAssembly dipakai **hanya** untuk ekstensi browser.
- **Bahasa:** hanya Rust (tokio untuk async, rustls untuk TLS). Satu-satunya pengecualian adalah glue JavaScript minimal yang diwajibkan WebExtension API (§5.12).
- **Bahasa UI:** Indonesia (`id`) sebagai bawaan, Inggris (`en`) sebagai cadangan. Semua teks UI berasal dari katalog i18n.
- **Mesin pengembangan:** Windows. Setiap perintah di berkas ini harus berjalan di PowerShell; tooling build ditulis dalam Rust (`xtask`), bukan skrip shell.
- **Batas cakupan:** UDM hanya mengunduh konten yang dapat diakses langsung oleh pengguna. Tidak ada pembobolan DRM, tidak ada dekripsi stream media berlisensi/terenkripsi, tidak ada bypass paywall atau captcha. Selamanya.

---

## 2. Mulai setiap sesi di sini

1. Baca `plan/STATUS.md` — milestone saat ini, tugas yang sedang dikerjakan, hambatan, commit hijau terakhir.
2. Baca `plan/execution-plan.md` — milestone M0…M8, tugas satu sesi, kriteria penerimaan.
3. Baca `plan/architecture.md` — peta crate, pelapisan, alur data.
4. Baca semua berkas di `plan/adr/` — keputusan yang sudah diterima mengikat sampai digantikan ADR baru.
5. Ikuti **protokol otonom di `plan/execution-plan.md` §6**. Ringkasnya:
   - ambil tugas berikutnya yang tidak terhalang, tandai *in progress* di `plan/STATUS.md`;
   - tulis tes yang gagal lebih dulu, lalu kodenya;
   - jalankan `cargo xtask ci` sampai hijau;
   - perbarui `plan/STATUS.md`, dokumentasi (kedua bahasa) dan `docs/en/parity-checklist-download-manager.md` bila perilaku berubah;
   - commit dengan pesan bahasa Inggris `<area>: <ringkasan imperatif>`;
   - berhenti dan catat pertanyaannya di `plan/STATUS.md` di bawah *Owner decisions needed* bila tugas membutuhkan keputusan pemilik yang tercantum di §6 rencana eksekusi.

`plan/` adalah memori kerja yang di-ignore git. Ditulis dalam bahasa Inggris.

---

## 3. Jangan pernah crash, jangan pernah kehilangan pekerjaan

Aturan ini berlaku untuk semua kode non-tes di crate produksi (semua kecuali `xtask`, `udm-testsrv`, dan kode `#[cfg(test)]`).

1. **Tanpa `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `unreachable!`, pengindeksan slice yang bisa panic, atau `unsafe`.** Error dikembalikan sebagai `Result<_, E>` dengan enum error bertipe per crate. `unreachable!` diganti varian error.
2. **Setiap crate produksi diawali header ini** (`lib.rs` / `main.rs`):
   ```rust
   #![forbid(unsafe_code)]
   #![deny(
       clippy::unwrap_used,
       clippy::expect_used,
       clippy::panic,
       clippy::todo,
       clippy::unimplemented,
       clippy::unreachable,
       clippy::indexing_slicing,
       clippy::dbg_macro,
       clippy::print_stdout,
       clippy::cast_possible_truncation,
       clippy::cast_sign_loss,
       clippy::arithmetic_side_effects,
       rust_2018_idioms,
       unused_must_use
   )]
   #![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::arithmetic_side_effects))]
   ```
   `udm-cli` boleh `allow(clippy::print_stdout)` hanya di modul keluarannya.
3. **Semua input dianggap berbahaya:** URL, respons HTTP/FTP, header, `Content-Disposition`, redirect, HTML dari site grabber, pesan native messaging, JSON port kontrol, teks clipboard, berkas daftar URL, berkas status di disk, dan berkas konfigurasi. Parse, validasi, batasi, baru gunakan.
4. **Batas keras berada di `crates/udm-core/src/limits.rs`** sebagai konstanta bernama (maks. redirect, byte dan jumlah header, panjang URL, panjang nama berkas, koneksi per unduhan, segmen, ukuran antrean, ukuran batch, kedalaman/halaman/byte halaman grabber, ukuran pesan native, ukuran pesan kontrol, ukuran berkas status, jumlah percobaan ulang, timeout). Tidak ada angka ajaib di tempat lain. Setiap batas punya tes `hostile_*` yang membuktikan batas itu ditegakkan.
5. **Aritmetika byte dicek.** Offset, panjang, dan rentang bertipe `u64` dan memakai `checked_*`/`saturating_*`; overflow adalah error, bukan wrap.
6. **Rekursi dibatasi atau dihilangkan.** Mengikuti redirect, site grabber, dan parser apa pun bersifat iteratif atau membawa penghitung kedalaman eksplisit yang dicek terhadap `limits.rs`.
7. **Simpan atomik.** Setiap berkas yang ditulis UDM untuk dirinya sendiri (status lanjutan, antrean, riwayat, pengaturan) ditulis ke `*.tmp` di direktori yang sama, di-`fsync`, lalu di-rename menimpa target; direktori induk di-`fsync` bila OS mendukung. Berkas status yang terpotong atau rusak dideteksi (versi skema + checksum) dan dilaporkan, tidak pernah dipakai diam-diam.
8. **Autosave.** Antrean, riwayat, dan pengaturan tersimpan otomatis saat berubah (debounce, ≤ 2 detik). Titik simpan lanjutan ditulis paling sedikit tiap 1 detik atau tiap 4 MiB per segmen, saat jeda, saat error, dan saat aplikasi ditutup.
9. **Pengaman panic.** Setiap tugas unduhan, setiap dispatch perintah, server kontrol, dan setiap frame UI berjalan di balik `std::panic::catch_unwind` (atau pengecekan panic `JoinHandle` untuk tugas tokio). Panic di satu unduhan menandai unduhan itu *Gagal (error internal)* dengan statusnya tetap tersimpan; tidak pernah menjatuhkan aplikasi. `panic = "abort"` dilarang di semua profil.
10. **Tes regresi untuk input berbahaya diberi nama `hostile_*`** (mis. `hostile_content_disposition_path_traversal`). `cargo xtask ci` menjalankannya sebagai grup terpisah yang selalu aktif.

---

## 4. Integritas dan keamanan unduhan (risiko produksi domain)

Setiap aturan di bawah dapat diuji dan punya tes di CI. Server injeksi gangguan lokal adalah `crates/udm-testsrv`; tes tidak pernah menyentuh internet.

### 4.1 Jangan pernah menyerahkan berkas rusak atau tidak lengkap
1. **Berkas sementara + status.** Selama mengunduh, data masuk ke `<nama-akhir>.udmpart` di folder tujuan; status lanjutan masuk ke direktori data sebagai `state/<download-id>.json` (atomik, §3.7). Nama akhir muncul **hanya** setelah finalisasi berhasil.
2. **Urutan ketahanan.** Data segmen di-flush (`sync_data`) *sebelum* titik simpan yang mencatatnya di-commit. Titik simpan tidak pernah mengklaim byte yang belum ada di disk.
3. **Validator sebelum melanjutkan.** Sebelum melanjutkan, UDM meminta ulang metadata dan membandingkan `ETag` kuat, `Last-Modified`, dan ukuran total dengan status yang tersimpan. Ada ketidakcocokan → berkas sementara dibuang dan unduhan dimulai ulang dari nol, dengan pemberitahuan yang terlihat (`download-restarted-remote-changed`). ETag lemah (`W/`) tidak dipercaya untuk melanjutkan; gunakan `Last-Modified` + ukuran, dan mulai ulang jika keduanya tidak ada.
4. **Respons Range diverifikasi.** Permintaan `Range` yang dijawab `200` (bukan `206`) tidak pernah ditambahkan: unduhan kembali ke satu koneksi dari offset 0. `Content-Range` harus sama persis dengan rentang yang diminta dan ukuran total; jika tidak, segmen gagal dan dicoba ulang. `Accept-Encoding: identity` dikirim untuk data berkas.
5. **Peta cakupan.** Setiap unduhan menyimpan peta cakupan rentang byte yang sudah selesai. Finalisasi mensyaratkan: tanpa celah, tanpa tumpang tindih, panjang cakupan == ukuran yang diharapkan, panjang berkas == ukuran yang diharapkan, dan (bila diberikan) checksum cocok. Baru setelah itu `.udmpart` di-rename ke nama akhir.
6. **Ukuran tidak diketahui.** Unduhan tanpa ukuran yang diketahui memakai satu koneksi, tidak bisa dipecah, dan ditandai "lanjutan tidak dijamin" di UI.
7. **Determinisme.** Perencana segmentasi dan aturan pemecahan ulang dinamis adalah fungsi murni dari (ukuran, koneksi, ukuran segmen minimum, peta cakupan). Tes properti (proptest) memeriksa: segmen menutup `[0, size)` tepat sekali, tidak pernah melebihi `limits::MAX_SEGMENTS`, dan tidak pernah lebih kecil dari `limits::MIN_SEGMENT_BYTES` kecuali ekor.
8. **Golden test injeksi gangguan.** Untuk setiap skenario, SHA-256 berkas yang selesai harus sama dengan SHA-256 fixture:
   koneksi putus di tengah segmen, server restart, aplikasi dimatikan paksa + dijalankan ulang (tes tingkat proses), ETag berubah di antara percobaan, `Range` diabaikan (200), `Content-Range` salah, `Content-Length` bohong (kurang dan lebih), server lambat, server mengirim byte lebih dari yang diminta, stream HTTP/2 di-reset.

### 4.2 Server berbahaya atau rusak
1. **Sanitasi nama berkas** (dari path URL, `filename`/`filename*` di `Content-Disposition` RFC 6266/5987, atau target redirect) adalah satu fungsi di `udm-proto` dengan tes `hostile_*` untuk: `..`, path absolut, huruf drive, `\` dan `/`, NUL dan karakter kontrol, override bidi Unicode (U+202E dll.), nama reserved Windows (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, dengan atau tanpa ekstensi, huruf besar/kecil apa pun), titik/spasi di akhir, nama lebih panjang dari `limits::MAX_FILENAME_BYTES`, nama kosong, dan ekstensi ganda berbahaya yang ditampilkan di UI (`invoice.pdf.exe`). Hasilnya selalu satu komponen path di dalam folder yang dipilih; berkas yang sudah ada tidak pernah ditimpa tanpa pilihan eksplisit (rename otomatis `nama (1).ext`).
2. **Redirect** dibatasi `limits::MAX_REDIRECTS`; loop dideteksi; penurunan HTTPS→HTTP ditolak kecuali pengguna mengizinkannya untuk unduhan itu.
3. **Header dan body dibatasi**: byte header, jumlah header, panjang baris (termasuk FTP), dan ukuran halaman grabber. Server yang mengirim header tanpa akhir atau bom dekompresi diputus dengan error.
4. **Verifikasi TLS tidak pernah dimatikan diam-diam.** rustls dengan verifikasi platform adalah bawaan. Pengecualian "terima sertifikat tidak valid" hanya bisa diberikan per host, lewat tindakan eksplisit pengguna yang menampilkan detail sertifikat, disimpan terlihat di pengaturan, dicatat di log, dan tidak pernah menjadi bawaan global atau bawaan CLI. FTPS mengikuti aturan yang sama.
5. **Timeout di mana-mana:** connect, handshake TLS, byte pertama, baca idle. Nilainya ada di `limits.rs`.

### 4.3 Ruang disk
1. Sebelum memulai (dan sebelum melanjutkan) UDM mengecek ruang kosong di volume tujuan terhadap sisa byte + margin keamanan (`limits::DISK_SAFETY_MARGIN_BYTES`). Tidak cukup → unduhan tidak dimulai, dengan pesan jelas yang menyebut volume dan kekurangannya.
2. Berkas dengan ukuran diketahui dialokasikan di muka (alokasi sungguhan, bukan sekadar `set_len` sparse, bila OS mendukung).
3. Error tulis (`ENOSPC`, kuota, drive dicabut) menjeda unduhan dengan status utuh dan pesan `error-disk-full` / `error-disk-write`; tidak pernah menghapus atau memotong data yang sudah tertulis. Tes menyuntikkan error ini lewat trait penulis.
4. Path yang lebih panjang dari batas lama Windows ditangani (prefiks `\\?\` lewat `std`/crate yang sudah diperiksa); path yang tidak bisa dibuat UDM dilaporkan sebelum unduhan dimulai.

### 4.4 Kredensial, cookie, dan privasi
1. Kata sandi, kredensial proxy, dan cookie yang disimpan hanya ada di keychain OS (Windows Credential Manager, macOS Keychain, Secret Service di Linux) lewat `udm-secrets`. Berkas status, pengaturan, riwayat, log, tangkapan layar, laporan crash, dan respons port kontrol menyimpan **referensi**, tidak pernah rahasianya.
2. Tipe yang memegang rahasia mengimplementasikan `Debug`/`Display` yang menyamarkan (`***`). Sebuah tes mencari fixture rahasia yang diketahui di log dan berkas status yang dihasilkan suite tes, dan gagal bila ada yang muncul.
3. Cookie **hanya** berasal dari ekstensi browser untuk permintaan spesifik yang ditangkap, lewat native messaging. UDM tidak pernah membaca basis data cookie browser atau berkas profil.
4. Tanpa telemetri. Tidak ada yang keluar dari mesin selain unduhan yang diminta pengguna.

### 4.5 Satuan dan waktu
Ukuran disimpan sebagai byte `u64` secara internal. UI menampilkan satuan biner dengan label IEC (KiB, MiB, GiB) dan kecepatan per detik, diformat oleh `udm-i18n` sesuai locale (`1,5 MiB/s` di `id`, `1.5 MiB/s` di `en`). Waktu disimpan dalam UTC dan ditampilkan dalam waktu lokal. Pilihan satuan hanya bisa diubah lewat ADR.

---

## 5. Hal yang tidak bisa ditawar

1. **Clean-room.** IDM boleh diamati sebagai kotak hitam (memakai produk publiknya, membaca halaman web publik, membandingkan perilaku). Jangan pernah mendekompilasi, membongkar (disassemble), mem-patch, mengekstrak, atau memeriksa biner, installer, data registry, kode ekstensi, format berkas, atau protokol jaringannya; jangan pernah menyalin teks, berkas bantuan, kata-kata dialog, ikon, logo, suara, atau tangkapan layarnya ke repo. Namanya hanya muncul sebagai pembanding fitur. Fakta yang tidak diketahui tentang IDM ditulis sebagai *perlu diverifikasi* di `plan/reference/domain.md`, tidak pernah dikarang.
2. **Aset dan `ATTRIBUTION.md`.** Setiap aset non-kode (ikon, gambar, font, suara, tabel MIME/kategori, berkas contoh, fixture tes yang tidak dihasilkan kode kita) punya baris di `ATTRIBUTION.md` berisi path, pembuat, sumber, lisensi, dan catatan, di commit yang sama dengan penambahannya. Tanpa baris → aset tidak boleh di-merge. Logo dan ikon IDM serta apa pun yang menyerupainya dilarang.
3. **Cakupan konten dan hukum.** Hanya konten yang dapat diakses langsung. Tidak ada penanganan DRM/Widevine/FairPlay/PlayReady, tidak ada dekripsi stream HLS/DASH terenkripsi, tidak ada pengakalan paywall, dinding login, captcha, atau batas laju, tidak ada fitur "perekam YouTube". Penangkapan media terbatas pada URL berkas langsung yang sudah ditampilkan halaman. Usulan di luar itu butuh ADR yang disetujui pemilik.
4. **UI dwibahasa lewat katalog i18n.** Semua string yang terlihat pengguna berada di berkas Fluent `crates/udm-i18n/locales/id/udm.ftl` dan `crates/udm-i18n/locales/en/udm.ftl`. Locale bawaan `id`, cadangan `en`. Tidak ada literal string tampilan di kode UI atau engine; error membawa kunci pesan + argumen. `cargo xtask i18n` gagal bila ada kunci hilang, kunci tak terpakai, atau kunci yang hanya ada di satu locale. Kata-kata UI `id` memakai bahasa Indonesia baku (KBBI), mis. *antrean*, *unduhan*, *jeda*, *lanjutkan*.
5. **Dokumentasi dwibahasa.** Dokumentasi pengguna dan pengembang berada di `docs/en/` dan `docs/id/` dengan nama berkas identik. `cargo xtask docs` gagal bila suatu berkas hanya ada di satu bahasa. `ROADMAP.md` dwibahasa dalam satu berkas.
6. **Semuanya adalah perintah.** Setiap aksi pengguna adalah `CommandSpec` yang terdaftar di `udm-engine` (`id`, argumen bertipe, kunci i18n label, kunci deskripsi, apakah butuh konfirmasi). Id perintah berformat `<area>.<verb>` huruf kecil snake case, mis. `download.add`, `download.pause`, `download.resume`, `download.remove`, `download.verify`, `queue.start`, `queue.stop`, `schedule.set`, `limit.set_global`, `limit.set_download`, `category.assign`, `capture.clipboard_toggle`, `batch.import`, `grabber.start`, `settings.set`. UI, CLI, port kontrol, host browser, dan penjadwal semuanya lewat dispatcher yang sama.
7. **Pemanggilan terprogram tanpa dialog.** Bila sebuah perintah menerima semua argumen wajib, perintah itu berjalan tanpa membuka dialog apa pun. Argumen kurang → GUI boleh membuka dialog; pemanggil headless (CLI, port kontrol, host native) menerima error bertipe. Permintaan konfirmasi dapat dipenuhi dengan `"confirm": true`.
8. **Pelapisan ditegakkan `cargo xtask layers`.** Sebuah crate hanya boleh bergantung pada crate di lapisan lebih rendah (tabel di `xtask/layers.toml`, dicerminkan di `plan/architecture.md`):

   | Lapisan | Crate | Catatan |
   |---|---|---|
   | L0 | `udm-core` | id, satuan, error, `limits.rs`, tipe `CommandSpec`. **Tanpa dependensi.** Harus bisa di-build untuk `wasm32-unknown-unknown`. |
   | L1 | `udm-i18n`, `udm-proto`, `udm-store` | i18n (Fluent); logika protokol murni (URL, header, `Content-Disposition`, sanitasi nama berkas, matematika rentang, perencana segmentasi, ekstraksi tautan); penyimpanan status/antrean/riwayat atomik. `udm-i18n` dan `udm-proto` harus bisa di-build untuk wasm. |
   | L2 | `udm-net`, `udm-secrets`, `udm-sys` | tokio/rustls/HTTP/FTP/proxy/auth; keychain OS; ruang disk, prealokasi, aksi daya, clipboard, notifikasi. |
   | L3 | `udm-engine` | unduhan, runtime segmentasi, antrean, penjadwal, pembatas kecepatan, kategori, checksum, registri + dispatcher perintah. |
   | L4 | `udm-control` | protokol kontrol JSON + server, parser `cmdline.input`. |
   | L5 | `udm-app` (bin `udm`, egui/eframe), `udm-cli` (bin `udm-cli`), `udm-native-host` (bin `udm-native-host`) | front-end tipis. |
   | Ext | `udm-ext` | inti ekstensi browser, `wasm32-unknown-unknown`; hanya boleh bergantung pada crate L0–L1 yang aman untuk wasm. |
   | Dev | `udm-testsrv`, `xtask` | tidak pernah menjadi dependensi L0–L5 kecuali sebagai `dev-dependency` (dan fitur opsional `dev-sample` di `udm-app`). |

9. **UI tipis.** `udm-app` menampilkan status dan mengirim perintah. Tidak ada jaringan, I/O berkas, parsing, atau aturan bisnis di kode UI. UI bisa diganti tanpa menyentuh L0–L4.
10. **Dependensi hanya dari allow-list** di `plan/adr/0001-dependencies.md` dan `deny.toml`. Lisensi: MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, Unicode-3.0, CC0, dan lisensi permisif sejenis. **GPL, LGPL, AGPL, SSPL, penggunaan MPL yang bersifat copyleft, dan lisensi "non-komersial" dilarang**, termasuk menyalin kode, mem-port algoritma baris per baris, atau membaca sumbernya untuk diimplementasikan ulang. Proyek di domain ini yang harus dihindari: aria2, GNU Wget, uGet, Xtreme Download Manager, JDownloader, Persepolis, KGet, FlashGot (verifikasi lisensi masing-masing di `plan/reference/domain.md`; bila ragu, anggap dilarang). Menambah dependensi membutuhkan amandemen ADR di commit yang sama.
11. **Satu bahasa pemrograman; wasm tidak boleh rusak.** Semua kode sumber di repo ini adalah Rust, kecuali glue WebExtension di `extension/glue/` (§5.12). Crate pihak ketiga yang mem-build C/asm bawaan (mis. penyedia kripto rustls) hanya diizinkan bila disebut di ADR 0001 beserta alasannya. `cargo xtask ci` mem-build `udm-ext`, `udm-core`, `udm-proto`, dan `udm-i18n` untuk `wasm32-unknown-unknown`; build wasm yang merah memblokir merge.
12. **Glue ekstensi.** `extension/glue/*.js` hanya memuat modul wasm dan meneruskan panggilan WebExtension API (`runtime`, `contextMenus`, `downloads`, `tabs`, `cookies` untuk permintaan yang ditangkap) ke dan dari Rust. Tidak ada parsing, penyaringan, atau keputusan di JS. Total glue ≤ 300 baris, dicek oleh `cargo xtask ci`. Manifest V3 untuk Chrome/Edge; varian manifest Firefox dihasilkan oleh `xtask`.
13. **Gerbang kualitas.** `cargo xtask ci` harus lulus sebelum setiap commit. Isinya: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, grup `hostile_*`, golden test injeksi gangguan, `cargo xtask layers`, `cargo xtask i18n`, `cargo xtask docs`, cek lisensi/allow-list (`cargo deny check` bila terpasang, jika tidak pemeriksa bawaan berbasis `cargo metadata`), build wasm, dan cek ukuran glue.

---

## 6. Glosarium domain (singkat)

| Inggris (kode/dokumen) | Indonesia (UI) | Arti |
|---|---|---|
| download | unduhan | Satu berkas yang sedang diambil, beserta statusnya. |
| segment | segmen | Rentang byte yang diambil oleh satu koneksi. |
| connection | koneksi | Satu stream TCP/TLS atau HTTP/2 yang dipakai untuk satu segmen. |
| dynamic segmentation | segmentasi dinamis | Saat satu segmen selesai, segmen tersisa terbesar dipecah agar koneksi yang menganggur tetap bekerja. |
| part file | berkas sementara (`.udmpart`) | Berkas data belum lengkap, di-rename saat finalisasi. |
| resume state / checkpoint | status lanjutan / titik simpan | Peta cakupan + validator yang disimpan. |
| validators | validator | `ETag`, `Last-Modified`, ukuran; bukti berkas di server tidak berubah. |
| pause / resume | jeda / lanjutkan | — |
| queue | antrean | Daftar berurutan yang diproses dengan batas jumlah serentak. |
| scheduler | penjadwal | Memulai/menghentikan antrean pada jam tertentu; aksi setelah selesai. |
| speed limiter | pembatas kecepatan | Token bucket, global dan per unduhan. |
| category | kategori | Kelompok jenis berkas dengan folder tujuan. |
| capture | tangkap | Mengambil tautan dari browser atau clipboard. |
| native messaging host | host native messaging | `udm-native-host`, jembatan antara ekstensi dan UDM. |
| site grabber | site grabber (penyedot situs) | Menjelajah halaman sampai kedalaman terbatas untuk mendaftar berkas. |
| batch download | unduh massal | Banyak URL dari daftar atau pola. |
| checksum | checksum | Hash MD5 (lemah, hanya pembanding) / SHA-256 dari berkas. |
| finalisation | finalisasi | Cek cakupan + ukuran + checksum, lalu rename. |

---

## 7. Menjalankan dan melihat aplikasi

Semua perintah berjalan di PowerShell pada Windows.

```powershell
# GUI dengan data contoh dan port kontrol agen
cargo run -p udm-app -- --sample --control 7878 --data-dir .udm-dev\agent-a

# Headless
cargo run -p udm-cli -- add http://127.0.0.1:8099/files/big.bin --dir .udm-dev\out --connections 8 --wait
cargo run -p udm-cli -- verify .udm-dev\out\big.bin --sha256 <hex>
cargo run -p udm-cli -- run --script .udm-dev\script.jsonl   # satu permintaan kontrol per baris

# Server tes injeksi gangguan secara terpisah
cargo run -p udm-testsrv -- --port 8099
```

- `--sample` (fitur `dev-sample`, aktif di build pengembangan, mati di paket rilis) menjalankan `udm-testsrv` di dalam proses pada port bebas dan mengisi antrean dengan unduhan yang mencakup setiap status (berjalan, dijeda, gagal, selesai, terjadwal, dimulai-ulang-karena-berubah). Tidak pernah menghubungi internet.
- `--control PORT` membuka server kontrol **hanya di 127.0.0.1**. Ditolak di build rilis kecuali `UDM_ALLOW_CONTROL=1` diset. Protokol: JSON permintaan/respons dipisah baris baru.
- `--data-dir` mengisolasi pengaturan, status, antrean, dan riwayat. Bawaannya direktori data OS.

Kirim permintaan dengan `cargo xtask ctl <PORT> '<json>'` (menghindari keanehan netcat di Windows):

```json
{"id":1,"method":"cmdline.input","params":{"text":"add http://127.0.0.1:8099/files/big.bin --connections 8"}}
{"id":2,"method":"engine.execute","params":{"command":"download.add","args":{"url":"http://127.0.0.1:8099/files/big.bin","dir":".udm-dev/out","connections":8,"category":"archives","start":true}}}
{"id":3,"method":"engine.execute","params":{"command":"download.pause","args":{"download_id":"d-000001"}}}
{"id":4,"method":"engine.execute","params":{"command":"limit.set_global","args":{"bytes_per_sec":524288}}}
{"id":5,"method":"engine.execute","params":{"command":"schedule.set","args":{"queue":"main","start_at":"23:00","stop_at":"05:00","after_done":"sleep"}}}
{"id":6,"method":"engine.query","params":{"what":"downloads"}}
{"id":7,"method":"ui.set_locale","params":{"locale":"en"}}
{"id":8,"method":"ui.screenshot","params":{"path":"target/shots/main-en.png"}}
```

Respons: `{"id":N,"ok":true,"result":{...}}` atau `{"id":N,"ok":false,"error":{"key":"error-...","args":{...}}}`. Kunci error adalah kunci i18n. Rahasia tidak pernah muncul di respons.

**Lihat UI setelah setiap perubahan UI:** ambil tangkapan layar dalam `id` dan `en` (`ui.set_locale` lalu `ui.screenshot`) dan periksa sebelum commit.

**Agen paralel:**
- satu git worktree per agen; tidak pernah dua agen di satu working tree;
- setiap agen memakai port `--control` sendiri, `--data-dir .udm-dev\<agen>` sendiri, dan membiarkan `udm-testsrv` memilih port bebas (`--port 0`);
- klaim tugas di `plan/STATUS.md` sebelum mulai; jangan mengerjakan tugas yang sudah diklaim agen lain;
- hanya satu agen pada satu waktu yang mengubah `xtask/layers.toml`, `deny.toml`, ADR, atau katalog Fluent; catat kuncinya di `plan/STATUS.md`.

---

## 8. Peta jalan

- `ROADMAP.md` — ringkasan milestone dwibahasa untuk manusia (M0…M8).
- `docs/en/gaps.md` / `docs/id/gaps.md` — kekurangan yang diketahui dan non-fitur yang disengaja (mis. tanpa DRM, tanpa perekam stream).
- `docs/en/parity-checklist-download-manager.md` / `docs/id/parity-checklist-download-manager.md` — perbandingan fitur per fitur dengan produk acuan, hanya perilaku yang teramati, tiap baris dengan status (`todo` / `partial` / `done` / `won't do`) dan tes yang membuktikannya.

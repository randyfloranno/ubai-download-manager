# PROMPT-AWAL — Sesi pertama Claude Code untuk UDM (Ubai Download Manager)

Tempel seluruh isi berkas ini ke Claude Code yang dibuka di folder repo ini (sudah berisi `AGENTS.md`, `AGENTS.id.md`, `CLAUDE.md`, `ATTRIBUTION.md`, `.gitignore`, dan `git init` sudah dijalankan).

---

## Peran

Kamu adalah arsitek dan engineer utama UDM (Ubai Download Manager). Kamu ahli Rust (workspace multi-crate, tokio, rustls, hyper/HTTP/2, egui/eframe, wasm-bindgen untuk WebExtension), rekayasa jaringan yang tahan gangguan (Range request, validator HTTP, FTP/FTPS, proxy), keamanan input berbahaya, i18n dengan Fluent, dan rekayasa clean-room. Kamu bekerja otonom, menulis tes lebih dulu, dan mematuhi `AGENTS.md` tanpa kecuali.

## Konteks

- Baca `AGENTS.md` sampai habis sebelum melakukan apa pun. Itu aturan kanonis repo ini; `AGENTS.id.md` hanya terjemahan.
- Produk: pengelola unduhan desktop (Windows, macOS, Linux) yang setara fitur dengan Internet Download Manager (IDM): unduhan multi-segmen dengan segmentasi dinamis, jeda/lanjutkan yang tahan putus koneksi dan restart, antrean dan penjadwal, pembatas kecepatan, kategori otomatis, integrasi browser (Chrome, Edge, Firefox) lewat ekstensi Rust→WASM + native messaging, tangkap clipboard, unduh massal, site grabber sederhana, proxy dan autentikasi, checksum MD5/SHA-256, riwayat, notifikasi. Protokol HTTP/1.1, HTTP/2, HTTPS, FTP, FTPS.
- Bahasa UI bawaan `id`, cadangan `en`. Mesin pengembangan Windows (PowerShell).
- Risiko produksi utama ada di `AGENTS.md` §4: berkas rusak/tidak lengkap, server berbahaya, disk penuh, kredensial dan cookie.
- Folder `plan/` di-ignore git; itu memori kerjamu dan ditulis dalam bahasa Inggris. Komentar kode dan pesan commit juga bahasa Inggris. Dokumentasi pengguna/pengembang dwibahasa di `docs/en/` dan `docs/id/`.
- Repo ini masih kosong selain berkas di atas.

Kerjakan empat fase berikut berurutan dalam sesi ini. Jangan lompat fase. Gunakan daftar tugas untuk melacak kemajuan.

---

## Fase 1 — Riset domain → `plan/reference/domain.md`

Tulis `plan/reference/domain.md` (bahasa Inggris) berisi:

1. **Fitur referensi yang teramati.** Daftar perilaku IDM yang diharapkan pengguna, hanya dari pengamatan kotak hitam dan halaman publik: alur tambah unduhan, panel progres per segmen, kategori, antrean, penjadwal, batas kecepatan, integrasi browser, tangkap media, unduh massal, site grabber, pengaturan koneksi/proxy. Setiap klaim yang tidak bisa kamu pastikan (versi, harga, nama menu, angka bawaan, jumlah koneksi maksimum, format internal) ditulis di bagian **"To verify"**, tidak dikarang.
2. **Standar dan format terbuka** yang relevan, dengan nomor RFC/spesifikasi: HTTP semantics dan Range request (RFC 9110), HTTP/1.1 (RFC 9112), HTTP/2 (RFC 9113), `Content-Disposition` (RFC 6266) dan `filename*` (RFC 8187), Digest auth (RFC 7616), Basic auth (RFC 7617), cookie (RFC 6265), FTP (RFC 959, 2389, 3659 untuk `SIZE`/`MDTM`/`REST STREAM`), FTPS (RFC 4217), SOCKS5 (RFC 1928/1929), proxy CONNECT, Chrome/Firefox native messaging dan Manifest V3 (catat batas ukuran pesan sebagai *to verify*), format daftar URL teks biasa. Pisahkan **format terbuka** dari **format proprietary** (mis. format antrean/ekspor IDM — tidak boleh dibaca atau ditiru; tandai "out of scope").
3. **Risiko produksi** dari `AGENTS.md` §4, masing-masing dengan skenario gangguan yang akan dibuat di `udm-testsrv`.
4. **Lisensi**: tabel proyek di domain yang sama dan lisensinya (aria2, GNU Wget, uGet, Xtreme Download Manager, JDownloader, Persepolis, KGet, FlashGot, curl, Motrix, Free Download Manager), status "forbidden / allowed as behaviour reference only", plus kolom "verified?" — jangan menyalin atau membaca kode mereka.
5. **Batas hukum/etika**: tanpa DRM, tanpa dekripsi stream terenkripsi, tanpa bypass paywall/captcha; daftar fitur IDM yang sengaja tidak ditiru karena alasan ini (masukkan juga ke `docs/*/gaps.md` di Fase 4).
6. **Istilah praktisi Indonesia**: padanan UI baku (unduhan, antrean, jeda, lanjutkan, tautan, kecepatan, segmen, berkas sementara, penjadwal) dan istilah yang lazim dibiarkan dalam bahasa Inggris (resume, site grabber, checksum, proxy).

## Fase 2 — Arsitektur dan ADR

1. `plan/architecture.md`: peta crate persis seperti tabel lapisan di `AGENTS.md` §5.8 (`udm-core`, `udm-i18n`, `udm-proto`, `udm-store`, `udm-net`, `udm-secrets`, `udm-sys`, `udm-engine`, `udm-control`, `udm-app`, `udm-cli`, `udm-native-host`, `udm-ext`, `udm-testsrv`, `xtask`), tanggung jawab tiap crate, diagram alur data (URL → probe → rencana segmen → koneksi → penulis berkas → titik simpan → finalisasi), model status unduhan (state machine dengan semua transisi, termasuk "restarted because remote changed" dan "paused: disk full"), format berkas status (`state/<download-id>.json` dengan versi skema + checksum), format id unduhan (`d-000001`, opaque), protokol kontrol, protokol native messaging (host `udm-native-host`, nama host sementara `dev.udm.native_host`), dan cara `--sample` bekerja.
2. `plan/adr/0001-dependencies.md`: allow-list dependensi dengan versi minor, lisensi, dan alasan. Kandidat untuk dievaluasi (verifikasi lisensi masing-masing sebelum dimasukkan): `tokio`, `hyper`, `hyper-util`, `http`, `http-body-util`, `h2`, `rustls`, `tokio-rustls`, `rustls-platform-verifier` atau `webpki-roots`, penyedia kripto rustls (`ring` vs `aws-lc-rs` — keduanya membawa C/asm; catat pengecualian ini dan pertimbangan build di Windows), `url`, `percent-encoding`, `serde`, `serde_json`, `fluent-bundle`, `unic-langid`, `eframe`/`egui`, `keyring`, `fs4` (ruang disk + prealokasi), `sha2`, `md-5`, `tracing`, `tracing-subscriber`, `clap`, `directories`, `arboard`, `notify-rust`, `keepawake`, `unicode-normalization`, `wasm-bindgen`, `js-sys`, `web-sys`, `proptest`, `tempfile`. Putuskan: FTP memakai crate (verifikasi lisensinya) atau klien minimal buatan sendiri di `udm-net`; riwayat memakai berkas JSON Lines atau basis data Rust murni (`redb`) — tanpa SQLite/C. Buat `deny.toml` yang selaras.
3. `plan/adr/0002-i18n.md`: Fluent, lokasi `crates/udm-i18n/locales/{id,en}/udm.ftl`, konvensi kunci (`<area>-<item>`, error `error-<area>-<what>`), fallback `en`, format angka/ukuran/waktu per locale (`1,5 MiB/s` vs `1.5 MiB/s`), pluralisasi, pemeriksaan `cargo xtask i18n`, aturan bahasa Indonesia baku (KBBI: *antrean*, bukan *antrian*).
4. `plan/adr/0003-download-integrity.md` (keputusan khas domain): strategi berkas sementara (satu berkas `.udmpart` praalokasi dengan tulis per offset vs berkas per segmen + penggabungan terverifikasi — pilih satu dan jelaskan), urutan `sync_data` → titik simpan, aturan validator untuk melanjutkan, perilaku saat `200` menjawab `Range`, algoritma segmentasi dinamis, nilai bawaan (koneksi per unduhan, `MIN_SEGMENT_BYTES`, interval titik simpan), kebijakan pengecualian sertifikat TLS, dan daftar skenario golden test.

## Fase 3 — Rencana eksekusi → `plan/execution-plan.md`

Pecah pekerjaan menjadi milestone M0 sampai M8. Setiap milestone dipecah menjadi tugas yang selesai dalam satu sesi (target ≤ ~400 baris perubahan), masing-masing dengan id (`M2-T03`), deskripsi, crate yang disentuh, **kriteria penerimaan**, dan **tes** yang membuktikannya (sebut nama tes, termasuk `hostile_*` dan golden test).

Garis besar milestone (perhalus, jangan kurangi cakupan):

- **M0 — Fondasi:** workspace, `rust-toolchain.toml`, header `#![deny(...)]` di semua crate produksi, `udm-core` (`limits.rs`, satuan, error, tipe `CommandSpec`), `udm-i18n` dengan katalog `id`/`en` awal, `xtask` (`ci`, `layers`, `i18n`, `docs`, `deny`/pemeriksa lisensi, `wasm`, `ctl`, `licences`, cek ukuran glue), `layers.toml`, `deny.toml`, kerangka `udm-app` (jendela egui berbahasa `id`, server kontrol `--control`, `ui.set_locale`, `ui.screenshot`, `--sample`, `--data-dir`), kerangka `udm-cli`, kerangka `udm-testsrv` (sajikan fixture deterministik dengan SHA-256 diketahui, dukung `Range`), kerangka `udm-ext` yang ter-build untuk wasm, `extension/glue/` minimal, `ROADMAP.md`, `docs/en|id/` (README pengembang, `gaps.md`, `parity-checklist-download-manager.md`).
- **M1 — Logika protokol murni (`udm-proto`):** parser URL dan header terbatas, `Content-Disposition`/`filename*`, sanitasi nama berkas lengkap dengan semua tes `hostile_*` dari `AGENTS.md` §4.2, matematika rentang dan `Content-Range`, perencana segmentasi + tes properti, peta cakupan.
- **M2 — Unduhan satu koneksi yang aman:** HTTP/1.1 + HTTPS (rustls), redirect terbatas, timeout, status atomik (`udm-store`), jeda/lanjutkan, validator, injeksi gangguan di `udm-testsrv` (putus koneksi, ETag berubah, `Range` diabaikan, `Content-Length` bohong, header tanpa akhir), golden test pertama.
- **M3 — Multi-segmen:** koneksi paralel, HTTP/2, segmentasi dinamis, finalisasi terverifikasi, cek ruang disk + prealokasi + error disk penuh tersimulasi, checksum MD5/SHA-256, tes kill-and-restart tingkat proses.
- **M4 — Engine lengkap:** antrean, penjadwal (jam mulai/berhenti, matikan/tidurkan setelah selesai lewat `udm-sys`), pembatas kecepatan global dan per unduhan, kategori + folder, riwayat + pencarian, notifikasi, pemulihan setelah crash, semua `CommandSpec` terdaftar.
- **M5 — UI desktop:** daftar unduhan, dialog tambah, properti dengan peta segmen, pengaturan, ganti bahasa, tangkap clipboard, impor daftar URL, tray; tangkapan layar `id`/`en` untuk tiap layar.
- **M6 — FTP/FTPS, proxy, autentikasi:** FTP (`REST`, `SIZE`, `MDTM`), FTPS eksplisit/implisit, proxy HTTP CONNECT dan SOCKS5, Basic/Digest, `udm-secrets` dengan keychain OS + tes kebocoran rahasia.
- **M7 — Integrasi browser:** `udm-native-host`, manifest host untuk Chrome/Edge/Firefox di Windows (registry, didaftarkan oleh installer/CLI UDM sendiri), macOS, Linux; `udm-ext` (menu konteks, tangkap unduhan, daftar tautan dan media langsung di halaman, cookie hanya untuk permintaan yang ditangkap); tes batas ukuran pesan.
- **M8 — Penyempurnaan:** site grabber terbatas, pola unduh massal, pengemasan/installer per OS, tanda tangan kode (keputusan pemilik), checklist paritas lengkap, dokumentasi pengguna dwibahasa.

Tambahkan **§6 Autonomous protocol** di `plan/execution-plan.md`, minimal:
1. Mulai sesi: baca `plan/STATUS.md`, rencana ini, `plan/architecture.md`, `plan/adr/`.
2. Ambil tugas pertama yang tidak terhalang dan belum diklaim; klaim di `plan/STATUS.md`.
3. Tulis tes yang gagal → implementasi → `cargo xtask ci` hijau.
4. Bila menyentuh UI: jalankan dengan `--sample --control`, ambil tangkapan layar `id` dan `en`, periksa.
5. Perbarui `plan/STATUS.md`, docs dwibahasa, dan checklist paritas; commit `<area>: <imperative summary>`.
6. Lanjut ke tugas berikutnya. Berhenti hanya jika: butuh keputusan pemilik, CI merah lebih dari tiga percobaan untuk sebab yang sama (catat diagnosis), atau tugas menuntut sesuatu yang melanggar `AGENTS.md`.
7. Jangan pernah melemahkan tes, batas, atau aturan lint untuk membuat CI hijau; perubahan aturan butuh ADR.

Tambahkan **daftar keputusan pemilik** (owner decisions), minimal: lisensi UDM sendiri (mis. MIT/Apache-2.0 ganda); nama reverse-DNS host native messaging dan ID ekstensi di toko; logo/ikon UDM; penyedia kripto rustls; satuan tampilan (IEC vs SI); jumlah koneksi bawaan dan maksimum; apakah pengecualian sertifikat TLS per host diizinkan sama sekali; apakah HLS/DASH tanpa enkripsi pernah didukung; mekanisme pembaruan otomatis; tanda tangan kode dan notarisasi; distribusi (installer, winget, Homebrew, paket Linux).

## Fase 4 — Status, dokumen, dan implementasi M0

1. Buat `plan/STATUS.md`: milestone aktif, tugas yang diklaim, commit hijau terakhir, hambatan, *Owner decisions needed*.
2. Buat `ROADMAP.md` (dwibahasa, satu berkas) dan `docs/en/` + `docs/id/` dengan nama berkas identik: `README.md` (cara build dan menjalankan di Windows), `gaps.md`, `parity-checklist-download-manager.md` (semua baris `todo` dulu, kolom: fitur, perilaku teramati, status, tes).
3. Implementasikan semua tugas M0 sesuai `plan/execution-plan.md`, berurutan, dengan protokol §6.
4. M0 selesai ketika `cargo xtask ci` hijau di Windows, termasuk build wasm `udm-ext`/`udm-core`/`udm-proto`/`udm-i18n`, dan perintah berikut berhasil:
   ```powershell
   cargo run -p udm-app -- --sample --control 7878 --data-dir .udm-dev\agent-a
   cargo xtask ctl 7878 '{"id":1,"method":"ui.set_locale","params":{"locale":"en"}}'
   cargo xtask ctl 7878 '{"id":2,"method":"ui.screenshot","params":{"path":"target/shots/m0-en.png"}}'
   ```
   Periksa tangkapan layarnya (`id` dan `en`).
5. Commit dengan pesan bahasa Inggris, mis. `build: bootstrap workspace, xtask gates and M0 skeleton`. Bila M0 terlalu besar untuk satu commit, commit per tugas.

---

## Batasan

- Patuhi `AGENTS.md` sepenuhnya; bila prompt ini dan `AGENTS.md` berbeda, `AGENTS.md` yang berlaku.
- Clean-room: IDM hanya pembanding perilaku. Jangan membongkar, mengekstrak, atau menyalin apa pun darinya; jangan menulis fakta spesifik IDM yang tidak kamu yakini — masukkan ke "To verify".
- Hanya dependensi berlisensi permisif yang tercatat di ADR 0001 dan `deny.toml`. Tidak ada kode, aset, atau terjemahan dari aria2, GNU Wget, uGet, Xtreme Download Manager, JDownloader, Persepolis, KGet, FlashGot, atau proyek GPL/LGPL/AGPL lain.
- Rust saja; JavaScript hanya di `extension/glue/` dan hanya sebagai penerus API (≤ 300 baris).
- Tanpa `unwrap`/`expect`/`panic!`/`unsafe` di kode produksi; semua teks UI dari katalog Fluent; tidak ada akses internet di tes (semua lewat `udm-testsrv`).
- Tanpa fitur DRM, dekripsi stream, atau bypass paywall/captcha, sekalipun terlihat mudah.
- Jangan melemahkan lint, tes, atau batas demi CI hijau.
- `plan/`, komentar kode, dan pesan commit dalam bahasa Inggris; dokumentasi di `docs/` dwibahasa.

## Definisi "selesai" untuk sesi ini

- [ ] `plan/reference/domain.md` lengkap, dengan bagian "To verify" dan tabel lisensi.
- [ ] `plan/architecture.md`, `plan/adr/0001-dependencies.md`, `plan/adr/0002-i18n.md`, `plan/adr/0003-download-integrity.md` ada dan saling konsisten dengan `AGENTS.md`.
- [ ] `plan/execution-plan.md` berisi M0–M8 terpecah menjadi tugas satu sesi dengan kriteria penerimaan dan tes, §6 protokol otonom, dan daftar keputusan pemilik.
- [ ] `plan/STATUS.md`, `ROADMAP.md`, `docs/en/` dan `docs/id/` (nama berkas identik) ada.
- [ ] Semua tugas M0 selesai; `cargo xtask ci` hijau di Windows, termasuk build wasm.
- [ ] `udm-app` berjalan dengan `--sample --control 7878`, menampilkan UI dalam `id` dan `en`, dan tangkapan layar keduanya sudah diperiksa.
- [ ] Semua perubahan sudah di-commit dengan pesan bahasa Inggris; `plan/STATUS.md` menunjuk tugas pertama M1 sebagai langkah berikutnya.

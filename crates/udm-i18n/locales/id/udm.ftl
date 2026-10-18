# UDM — katalog bahasa Indonesia (bawaan).
## dynamic-prefixes: category- power- status- unit- cmd- after-done- proxy-kind-
## not-keys: error-x proxy-user proxy-pass proxy-authorization proxy-connection

app-title = UDM — Ubai Download Manager

## Status unduhan
status-queued = Dalam antrean
status-scheduled = Terjadwal
status-probing = Menghubungkan
status-downloading = Mengunduh
status-paused = Dijeda
status-paused-disk-full = Dijeda: disk penuh
status-paused-disk-write = Dijeda: galat tulis disk
status-paused-shutdown = Dijeda (aplikasi ditutup)
status-paused-schedule = Dijeda oleh penjadwal
status-finalising = Finalisasi
status-completed = Selesai
status-failed = Gagal

## Kategori
category-documents = Dokumen
category-video = Video
category-music = Musik
category-programs = Program
category-archives = Arsip
category-general = Umum

## Tindakan setelah selesai
after-done-none = Tidak melakukan apa pun
after-done-exit = Keluar dari UDM
after-done-shutdown = Matikan komputer
after-done-sleep = Tidur (sleep)
after-done-hibernate = Hibernasi

## Jenis proksi
proxy-kind-none = Tanpa proksi
proxy-kind-http = Proksi HTTP
proxy-kind-socks5 = Proksi SOCKS5

## Pemberitahuan
download-restarted-remote-changed = Berkas di server berubah, sehingga unduhan dimulai ulang dari awal.
notice-resume-state-corrupt = Progres yang tersimpan rusak; unduhan dimulai lagi.
notice-history-lines-skipped = { $count } entri riwayat yang rusak dilewati.
notice-queue-finished = Antrean "{ $queue }" telah selesai.
notice-verify-mismatch = Checksum { $name } tidak cocok.

## Galat perintah
error-command-too-many-args = Argumen terlalu banyak (maksimum { $max }).
error-command-unknown-arg = Perintah { $command } tidak punya argumen "{ $arg }".
error-command-bad-arg = Nilai argumen "{ $arg }" untuk { $command } tidak valid.
error-command-missing-arg = Perintah { $command } memerlukan argumen "{ $arg }".
error-command-needs-confirmation = Perintah { $command } memerlukan konfirmasi.
error-command-unknown = Perintah { $command } tidak dikenal.
error-command-bad-args = Argumen perintah harus berupa objek JSON.
error-state-coverage-overlap = Progres yang tersimpan rusak (rentang tumpang-tindih).

## Galat mesin unduhan
error-engine-not-found = Unduhan { $id } tidak ada.
error-engine-queue-not-found = Antrean "{ $queue }" tidak ada.
error-engine-queue-exists = Antrean "{ $queue }" sudah ada.
error-engine-too-many-queues = Jumlah antrean terlalu banyak.
error-engine-queue-main = Antrean utama tidak dapat dihapus.
error-engine-queue-full = Antrean sudah penuh.
error-engine-category-not-found = Kategori "{ $category }" tidak ada.
error-engine-batch-too-large = Daftar berisi terlalu banyak alamat.
error-engine-job-not-found = Tugas penyedot situs { $job } tidak ada.
error-engine-cannot-pause = Unduhan ini tidak dapat dijeda sekarang.
error-engine-already-completed = Unduhan ini sudah selesai.
error-engine-download-active = Hentikan unduhan terlebih dahulu.
error-engine-not-completed = Unduhan ini belum selesai.
error-engine-internal = Galat internal. Status unduhan tetap disimpan.
error-engine-no-free-name = Tidak ada nama berkas yang tersedia untuk { $name }.
error-engine-create-dir = Folder { $path } tidak dapat dibuat.
error-engine-incomplete = Unduhan belum lengkap; berkas tidak diserahkan.
error-engine-size-mismatch = Ukuran berkas salah: seharusnya { $expected }, didapat { $actual }.
error-engine-checksum-mismatch = Checksum tidak cocok: seharusnya { $expected }, didapat { $actual }.
error-engine-read-file = Berkas { $path } tidak dapat dibaca.
error-settings-unknown = Pengaturan { $name } tidak dikenal.
error-settings-bad-value = Nilai pengaturan { $name } tidak valid.
error-schedule-bad-time = Waktu "{ $value }" tidak valid (gunakan JJ:MM).
error-schedule-bad-day = Hari "{ $value }" tidak valid.

## Galat jaringan
error-net-connect = Tidak dapat terhubung ke { $host }.
error-net-timeout = Koneksi habis waktu setelah { $seconds } detik.
error-net-tls = Koneksi aman ke { $host } gagal.
error-net-tls-untrusted = Sertifikat { $host } tidak tepercaya (SHA-256 { $fingerprint }).
error-net-http-status = Server menjawab dengan status { $status }.
error-net-too-many-redirects = Terlalu banyak pengalihan (lebih dari { $max }).
error-net-redirect-loop = Server mengalihkan secara berputar.
error-net-downgrade-refused = Pengalihan dari alamat aman ke alamat tidak aman ditolak.
error-net-redirect-invalid = Server mengirim pengalihan yang tidak valid.
error-net-header-too-large = Header respons server terlalu besar (batas { $max }).
error-net-too-many-headers = Server mengirim terlalu banyak header (batas { $max }).
error-net-content-encoding = Server memampatkan data berkas ({ $encoding }); ini tidak didukung.
error-net-short-body = Koneksi terputus terlalu awal setelah { $received }.
error-net-extra-bytes = Server mengirim lebih banyak data daripada yang diminta.
error-net-bad-content-range = Server mengirim rentang bita yang salah.
error-net-remote-changed = Berkas di server telah berubah.
error-net-auth-required = Server memerlukan nama pengguna dan kata sandi.
error-net-auth-failed = Nama pengguna atau kata sandi ditolak.
error-net-proxy = Proksi menolak koneksi (kode { $code }).
error-net-proxy-auth = Proksi menolak kredensial.
error-net-ftp = Galat server FTP { $code }: { $text }
error-net-unsupported-scheme = Jenis alamat tidak didukung: { $scheme }.
error-net-invalid-request = Permintaan tidak valid.
error-net-io = Terjadi galat baca/tulis jaringan.
error-net-stream = Aliran data terputus.
error-net-protocol = Server melanggar protokol.
error-net-not-html = Halaman bukan HTML ({ $type }).

## Galat protokol
error-proto-url-too-long = Alamat terlalu panjang ({ $len } karakter, maksimum { $max }).
error-proto-url-invalid = Alamat tidak valid.
error-proto-url-scheme = Jenis alamat tidak didukung: { $scheme }.
error-proto-url-no-host = Alamat tidak memiliki nama host.
error-proto-redirect-invalid = Alamat pengalihan tidak valid.
error-proto-header-value = Nilai header terlalu panjang atau tidak valid.
error-proto-content-range-missing = Server tidak mengirim rentang bita.
error-proto-content-range-invalid = Server mengirim rentang bita yang tidak valid.
error-proto-content-range = Rentang bita salah: seharusnya { $expected }, didapat { $actual }.
error-proto-auth-challenge = Permintaan masuk dari server tidak valid.
error-proto-ftp-line-too-long = Baris balasan FTP terlalu panjang (maksimum { $max }).
error-proto-ftp-too-many-lines = Balasan FTP berisi terlalu banyak baris (maksimum { $max }).
error-proto-ftp-reply = Server FTP mengirim balasan yang tidak valid.
error-proto-url-list-too-large = Daftar alamat terlalu besar ({ $bytes }, maksimum { $max }).
error-proto-batch-too-large = Terlalu banyak alamat ({ $count }, maksimum { $max }).
error-proto-batch-pattern = Pola alamat tidak valid.
error-proto-checksum-invalid = Checksum tidak valid (gunakan sha256:<hex> atau md5:<hex>).

## Galat penyimpanan
error-store-io = { $path } tidak dapat dibaca atau ditulis.
error-store-corrupt = Berkas { $path } rusak.
error-store-kind-mismatch = Isi berkas { $path } salah.
error-store-schema-unsupported = Berkas { $path } berasal dari versi UDM yang lebih baru.
error-store-parse = Berkas { $path } tidak dapat dibaca.
error-store-too-large = Berkas { $path } terlalu besar.

## Galat sistem
error-disk-full = Disk penuh saat menulis { $path }.
error-disk-write = Galat saat menulis ke disk: { $path }.
error-sys-io = Galat berkas di { $path }.
error-sys-already-exists = { $path } sudah ada.
error-sys-path-too-long = Jalur terlalu panjang: { $path }.
error-sys-file-too-large = Berkas terlalu besar.
error-sys-unsupported = Tidak didukung pada sistem ini.
error-sys-clipboard = Papan klip tidak dapat dibaca.
error-sys-notify = Pemberitahuan tidak dapat ditampilkan.
error-sys-power = Tindakan daya gagal.
error-sys-not-enough-space = Ruang kosong di { $volume } tidak cukup: perlu { $needed }, tersedia { $available } (kurang { $shortfall }).

## Galat penyimpanan rahasia
error-secrets-unavailable = Gantungan kunci sistem tidak tersedia.
error-secrets-backend = Gantungan kunci sistem melaporkan galat.
error-secrets-invalid-ref = Referensi gantungan kunci tidak valid.
error-secrets-too-long = Rahasia terlalu panjang.

## Perintah (label dan deskripsi)
cmd-download-add = Tambah unduhan
cmd-download-add-desc = Tambahkan unduhan baru dari sebuah alamat.
cmd-download-pause = Jeda
cmd-download-pause-desc = Jeda unduhan; unduhan dapat dilanjutkan nanti.
cmd-download-resume = Lanjutkan
cmd-download-resume-desc = Lanjutkan unduhan yang dijeda atau gagal.
cmd-download-remove = Hapus
cmd-download-remove-desc = Hapus unduhan dari daftar.
cmd-download-restart = Mulai ulang
cmd-download-restart-desc = Unduh ulang berkas dari awal.
cmd-download-verify = Periksa checksum
cmd-download-verify-desc = Hitung checksum berkas yang sudah selesai.
cmd-download-set-connections = Atur koneksi
cmd-download-set-connections-desc = Ubah jumlah koneksi sebuah unduhan.
cmd-download-pause-all = Jeda semua
cmd-download-pause-all-desc = Jeda semua unduhan yang berjalan.
cmd-download-resume-all = Lanjutkan semua
cmd-download-resume-all-desc = Lanjutkan semua unduhan yang dijeda.
cmd-download-remove-completed = Hapus yang selesai
cmd-download-remove-completed-desc = Hapus semua unduhan yang selesai dari daftar.
cmd-queue-start = Mulai antrean
cmd-queue-start-desc = Mulai memproses antrean.
cmd-queue-stop = Hentikan antrean
cmd-queue-stop-desc = Berhenti memproses antrean.
cmd-queue-create = Antrean baru
cmd-queue-create-desc = Buat antrean baru.
cmd-queue-delete = Hapus antrean
cmd-queue-delete-desc = Hapus antrean; unduhannya dipindahkan ke antrean utama.
cmd-queue-set-concurrency = Atur unduhan serentak
cmd-queue-set-concurrency-desc = Berapa unduhan antrean yang berjalan bersamaan.
cmd-queue-move-item = Pindahkan dalam antrean
cmd-queue-move-item-desc = Pindahkan unduhan ke antrean atau posisi lain.
cmd-schedule-set = Atur jadwal
cmd-schedule-set-desc = Mulai dan hentikan antrean pada waktu tertentu.
cmd-schedule-clear = Hapus jadwal
cmd-schedule-clear-desc = Hapus jadwal sebuah antrean.
cmd-limit-set-global = Batas kecepatan global
cmd-limit-set-global-desc = Batasi kecepatan unduh total (0 = tanpa batas).
cmd-limit-set-download = Batas kecepatan unduhan
cmd-limit-set-download-desc = Batasi kecepatan satu unduhan (0 = tanpa batas).
cmd-category-assign = Atur kategori
cmd-category-assign-desc = Masukkan unduhan ke sebuah kategori.
cmd-category-set-folder = Folder kategori
cmd-category-set-folder-desc = Pilih folder sebuah kategori.
cmd-category-set-extensions = Jenis berkas kategori
cmd-category-set-extensions-desc = Pilih jenis berkas yang termasuk dalam kategori.
cmd-capture-clipboard-toggle = Tangkap papan klip
cmd-capture-clipboard-toggle-desc = Pantau papan klip untuk tautan unduhan.
cmd-capture-clipboard-text = Tambah dari papan klip
cmd-capture-clipboard-text-desc = Tambahkan tautan unduhan yang ditemukan dalam teks.
cmd-capture-browser = Tangkap dari peramban
cmd-capture-browser-desc = Tambahkan unduhan yang ditangkap oleh ekstensi peramban.
cmd-batch-import = Unduh massal
cmd-batch-import-desc = Tambahkan banyak unduhan dari daftar alamat.
cmd-batch-pattern = Unduh massal dari pola
cmd-batch-pattern-desc = Tambahkan unduhan dari pola alamat seperti berkas[1-10].zip.
cmd-grabber-start = Penyedot situs
cmd-grabber-start-desc = Cari berkas untuk diunduh di sebuah halaman web.
cmd-grabber-add-results = Tambah hasil penyedotan
cmd-grabber-add-results-desc = Tambahkan berkas yang ditemukan oleh penyedot situs.
cmd-settings-set = Ubah pengaturan
cmd-settings-set-desc = Ubah satu pengaturan.
cmd-tls-allow-host = Percayai sertifikat
cmd-tls-allow-host-desc = Terima sertifikat satu host setelah memeriksa detailnya.
cmd-tls-revoke-host = Cabut kepercayaan sertifikat
cmd-tls-revoke-host-desc = Berhenti menerima pengecualian sertifikat sebuah host.
cmd-proxy-set = Proksi
cmd-proxy-set-desc = Atur server proksi.
cmd-auth-set-site = Login situs
cmd-auth-set-site-desc = Simpan nama pengguna dan kata sandi untuk sebuah situs.
cmd-auth-remove-site = Hapus login situs
cmd-auth-remove-site-desc = Lupakan login tersimpan sebuah situs.
cmd-history-clear = Bersihkan riwayat
cmd-history-clear-desc = Hapus semua entri riwayat.
cmd-history-remove = Hapus dari riwayat
cmd-history-remove-desc = Hapus satu entri riwayat.

## Control port and command line
error-control-bad-json = Permintaan bukan JSON yang valid.
error-control-bad-request = Format permintaan salah.
error-control-message-too-large = Pesan lebih besar dari { $max } bita.
error-control-no-ui = Tidak ada jendela untuk permintaan ini.
error-control-unknown-method = Metode tidak dikenal: { $method }.
error-control-bad-query = Kueri tidak dikenal: { $what }.
error-control-missing-param = Parameter tidak ada: { $param }.
error-control-unauthorized = Permintaan tidak diizinkan.
error-control-forbidden = Metode ini tidak diizinkan di titik akhir ini.
error-control-busy = UDM sedang sibuk; coba lagi sebentar lagi.
error-control-connect = Tidak dapat memakai port lokal { $port }.
error-control-io = Koneksi ke UDM gagal.
error-control-random = Tidak dapat membuat token acak yang aman.
error-cmdline-too-long = Baris perintah lebih panjang dari { $max } bita.
error-cmdline-unterminated-quote = Ada tanda kutip yang tidak ditutup.
error-cmdline-too-many-tokens = Baris perintah berisi lebih dari { $max } kata.
error-cmdline-empty = Baris perintah kosong.
error-cmdline-extra-word = Kata tidak terduga: { $word }.

## Application
error-app-control-refused = Port kendali dinonaktifkan pada versi rilis (atur UDM_ALLOW_CONTROL=1).
error-app-sample-unavailable = Data contoh tidak disertakan dalam versi ini.
error-app-runtime = Tidak dapat menjalankan proses latar.
error-app-window = Tidak dapat membuka jendela.
error-app-sample-server = Tidak dapat menjalankan server contoh.
error-ui-timeout = Jendela tidak menjawab tepat waktu.
error-ui-screenshot-path = Jalur tangkapan layar harus berkas .png tanpa "..".
error-ui-screenshot-write = Tidak dapat menyimpan tangkapan layar.
error-ui-unknown-dialog = Dialog tidak dikenal: { $dialog }.
error-ui-bad-locale = Bahasa tidak dikenal: { $locale }.
error-ui-frame = Terjadi kesalahan saat menggambar jendela; tampilan telah diatur ulang.

## Main window
ui-filter-all = Semua unduhan
ui-filter-unfinished = Belum selesai
ui-filter-finished = Selesai
ui-section-categories = Kategori
ui-section-queues = Antrean
ui-queue-main = Antrean utama
ui-col-name = Nama berkas
ui-col-size = Ukuran
ui-col-progress = Kemajuan
ui-col-speed = Kecepatan
ui-col-eta = Sisa waktu
ui-col-status = Status
ui-col-added = Ditambahkan
ui-empty-list = Belum ada unduhan di sini.
ui-no-selection = Pilih unduhan untuk melihat detailnya.
ui-unknown-size = Tidak diketahui
ui-status-bar = { $active } aktif dari { $total } · { $speed }
ui-limit-global = Batas kecepatan: { $limit }
ui-limit-global-none = Tanpa batas kecepatan
ui-limit-none = Tidak ada
ui-detail-url = Alamat
ui-detail-folder = Folder
ui-detail-connections = Koneksi
ui-detail-limit = Batas kecepatan
ui-detail-downloaded = Terunduh
ui-detail-resume = Dapat dilanjutkan
ui-resume-yes = Didukung
ui-resume-no = Tidak dijamin
ui-dangerous-name = Peringatan: "{ $name }" menyembunyikan program di balik ekstensi dokumen.
ui-notice-completed = Unduhan selesai: { $name }
ui-verify-match = { $algo } cocok: { $hash }
ui-verify-mismatch = { $algo } TIDAK cocok: { $hash }
ui-verify-result = { $algo }: { $hash }
ui-unit-kib-s = {" "}KiB/s
ui-hint-auto = Otomatis

## Buttons
ui-btn-add = Tambah
ui-btn-cancel = Batal
ui-btn-close = Tutup
ui-btn-apply = Terapkan
ui-btn-save = Simpan

## Dialogs
ui-add-title = Tambah unduhan
ui-add-url = Alamat
ui-add-folder = Folder
ui-add-file-name = Nama berkas
ui-add-connections = Koneksi
ui-add-category = Kategori
ui-add-checksum = Checksum
ui-add-start-now = Mulai sekarang
ui-settings-title = Pengaturan
ui-settings-language = Bahasa
ui-lang-id = Bahasa Indonesia
ui-lang-en = English
ui-settings-download-dir = Folder unduhan
ui-settings-max-concurrent = Unduhan bersamaan
ui-settings-global-limit = Batas kecepatan global
ui-settings-subfolders = Simpan tiap kategori di foldernya sendiri
ui-settings-notify = Beri tahu saat unduhan selesai
ui-settings-close-to-tray = Tetap jalankan UDM di baki sistem saat jendela ditutup
ui-tray-show = Tampilkan UDM
ui-tray-quit = Keluar
ui-tray-still-running = UDM masih berjalan di baki sistem.
ui-schedule-queue = Antrean
ui-schedule-start = Mulai pukul
ui-schedule-stop = Berhenti pukul
ui-schedule-days = Hari
ui-schedule-days-hint = mon,tue,… (kosong = setiap hari)
ui-schedule-after = Setelah selesai
ui-schedule-hint = Waktu setempat, format 24 jam JJ:MM.
ui-batch-hint = Satu alamat per baris, atau satu pola seperti https://example.org/file[1-10].zip
ui-grab-depth = Kedalaman
ui-grab-extensions = Jenis berkas
ui-grab-same-host = Tetap di situs yang sama
ui-grab-start = Cari
ui-grab-add = Tambah berkas yang ditemukan
ui-grab-running = Mencari… { $count } berkas di { $pages } halaman sejauh ini
ui-grab-found = Ditemukan { $count } berkas di { $pages } halaman
ui-remove-title = Hapus unduhan
ui-remove-question = Hapus "{ $name }" dari daftar?
ui-remove-delete-file = Hapus juga berkasnya dari disk
ui-properties-title = Properti
ui-captured-title = Tautan tertangkap
ui-captured-question = Ditemukan { $count } tautan unduhan di papan klip. Tambahkan?

## Native messaging host

error-native-too-large = Pesan dari peramban terlalu besar.
error-native-truncated = Pesan dari peramban terpotong.
error-native-io = Tidak dapat berkomunikasi dengan peramban.
error-native-bad-json = Pesan dari peramban bukan JSON yang valid.
error-native-bad-message = Pesan dari peramban memiliki kolom tidak valid: { $field }.
error-native-ipc-unreadable = Tidak dapat membaca berkas koneksi UDM.
error-native-app-not-running = UDM tidak berjalan. Jalankan UDM lalu coba lagi.
error-native-internal = Kesalahan internal pada jembatan peramban.

## CLI
error-cli-app-running = UDM sedang berjalan pada folder data ini. Gunakan --control PORT untuk terhubung.
error-cli-script-read = Skrip { $path } tidak dapat dibaca.
error-cli-script-too-large = Skrip lebih besar dari { $max } bita.
error-cli-script-failed = Satu atau beberapa baris skrip gagal.
error-cli-checksum-missing = Berikan --sha256 atau --md5.
error-cli-checksum-mismatch = Checksum { $name } tidak cocok.
error-cli-download-failed = Unduhan gagal.
error-cli-extension-id = { $id } bukan id ekstensi yang sah (32 huruf a–p).
error-cli-host-missing = Program host native tidak ditemukan: { $path }
error-cli-register-write = Tidak dapat menulis { $path }.
error-cli-register-reg = Tidak dapat memperbarui registri Windows.

## Ekstensi peramban
ext-menu-link = Unduh tautan dengan UDM
ext-menu-media = Unduh media ini dengan UDM
ext-sent = Dikirim ke UDM.
ext-host-missing = UDM belum terpasang atau penghubung perambannya belum terdaftar.
ext-reply-invalid = UDM mengirim jawaban yang tidak dipahami ekstensi ini.

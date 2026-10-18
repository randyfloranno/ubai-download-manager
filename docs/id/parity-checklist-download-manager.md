# Daftar periksa kesetaraan fitur (hanya perilaku yang diamati)

| Fitur | Status | Tes pembukti |
|---|---|---|
| Unduhan multisegmen, segmentasi dinamis | done | `golden_plain_multi_segment`, `plan_covers_gaps_exactly_once` (proptest) |
| HTTP/2 (ALPN), pemulihan reset stream | done | `golden_http2_stream_reset`, `tls_h2_negotiation_ranges_and_reset` |
| Lanjutkan setelah jeda | done | `golden_pause_resume` |
| Lanjutkan setelah aplikasi dimulai ulang / crash | done | `golden_app_restart_resumes`, `golden_crash_without_shutdown_resumes_same_part_file` |
| Lanjutkan setelah proses dimatikan paksa | done | `golden_app_kill_restart` (tingkat proses, `udm-cli`) |
| Mulai ulang bila berkas di server berubah | done | `golden_etag_change_between_attempts_restarts` |
| Server yang mengabaikan atau berbohong soal rentang / panjang | done | `golden_range_ignored_falls_back_to_single`, `golden_wrong_content_range_never_delivers`, `golden_content_length_lies` |
| Verifikasi checksum (MD5 / SHA-256) | done | `golden_checksum_verified`, `verify_matches_and_mismatches` |
| Disk penuh menjeda dengan status utuh | done | `golden_disk_full_pauses_with_state` |
| Ganti nama otomatis bila nama bentrok | done | `same_name_is_auto_renamed` |
| Antrean dengan batas konkurensi | done | `hostile_queue_size_is_bounded`, `hostile_too_many_queues_refused` |
| Penjadwal (waktu mulai/berhenti, matikan/tidur setelah selesai) | done | `schedule_window`, `hostile_schedule_bad_values` |
| Batas kecepatan global dan per unduhan | done | `limiter_bounds_rate` |
| Kategori menurut jenis berkas beserta folder | done | `hostile_category_table_within_limits` |
| Riwayat dan pencarian | done | `rewrite_keeps_newest_max_history_records`, `hostile_history_*` |
| Tangkap tautan dari papan klip | done | `hostile_clipboard_text_bounded` |
| Unduh massal (daftar / pola) | done | `hostile_batch_pattern_too_large`, `hostile_batch_pattern_invalid` |
| Site grabber (terbatas) | done | `hostile_grabber_depth_is_capped`, `hostile_grabber_page_count_is_capped` |
| FTP / FTPS (eksplisit, implisit), lanjutkan dengan REST | done | `ftp_retr_with_rest_and_drop`, `ftps_explicit_probe_and_fetch`, `ftps_implicit_probe_and_fetch` |
| Proksi HTTP dan SOCKS5 dengan autentikasi | done | `http_connect_proxy_with_auth`, `socks5_tunnel_with_auth` |
| Autentikasi Basic dan Digest | done | `digest_rfc7616_vectors`, `digest_auth_with_credentials_fetches_data` |
| Kredensial hanya di keychain sistem operasi | done | `no_secret_in_logs_or_state` |
| Integrasi peramban (Chrome, Edge, Firefox) | done | `hostile_ext_events_are_ignored`, `hostile_native_message_too_large`, `manifest_json_shape` |
| Baris perintah dan skrip | done | `local_backend_runs_script_lines`, `cli_parses` |
| Ikon baki sistem (tampilkan, jeda semua, lanjutkan semua, keluar; tutup ke baki) | partial (hanya Windows) | `tray_labels_exist_in_both_locales`; pemeriksaan manual di Windows |
| Installer dan penandatanganan kode | todo | keputusan pemilik; `cargo xtask dist` membuat folder |
| DRM / perekaman stream | won't do | — |

# Feature parity checklist (observed behaviour only)

| Feature | Status | Proving test |
|---|---|---|
| Multi-segment download, dynamic segmentation | done | `golden_plain_multi_segment`, `plan_covers_gaps_exactly_once` (proptest) |
| HTTP/2 (ALPN), stream reset recovery | done | `golden_http2_stream_reset`, `tls_h2_negotiation_ranges_and_reset` |
| Resume after pause | done | `golden_pause_resume` |
| Resume after app restart / crash | done | `golden_app_restart_resumes`, `golden_crash_without_shutdown_resumes_same_part_file` |
| Resume after the process is killed | done | `golden_app_kill_restart` (process-level, `udm-cli`) |
| Restart when remote file changed | done | `golden_etag_change_between_attempts_restarts` |
| Servers that ignore or lie about ranges / length | done | `golden_range_ignored_falls_back_to_single`, `golden_wrong_content_range_never_delivers`, `golden_content_length_lies` |
| Checksum verification (MD5 / SHA-256) | done | `golden_checksum_verified`, `verify_matches_and_mismatches` |
| Disk full pauses with state intact | done | `golden_disk_full_pauses_with_state` |
| Auto-rename on name clash | done | `same_name_is_auto_renamed` |
| Queues with concurrency limit | done | `hostile_queue_size_is_bounded`, `hostile_too_many_queues_refused` |
| Scheduler (start/stop time, shut down/sleep after completion) | done | `schedule_window`, `hostile_schedule_bad_values` |
| Global and per-download speed limit | done | `limiter_bounds_rate` |
| Categories by file type with folders | done | `hostile_category_table_within_limits` |
| History and search | done | `rewrite_keeps_newest_max_history_records`, `hostile_history_*` |
| Clipboard link capture | done | `hostile_clipboard_text_bounded` |
| Batch download (list / pattern) | done | `hostile_batch_pattern_too_large`, `hostile_batch_pattern_invalid` |
| Site grabber (bounded) | done | `hostile_grabber_depth_is_capped`, `hostile_grabber_page_count_is_capped` |
| FTP / FTPS (explicit, implicit), resume with REST | done | `ftp_retr_with_rest_and_drop`, `ftps_explicit_probe_and_fetch`, `ftps_implicit_probe_and_fetch` |
| HTTP and SOCKS5 proxy with authentication | done | `http_connect_proxy_with_auth`, `socks5_tunnel_with_auth` |
| Basic and Digest authentication | done | `digest_rfc7616_vectors`, `digest_auth_with_credentials_fetches_data` |
| Credentials only in the OS keychain | done | `no_secret_in_logs_or_state` |
| Browser integration (Chrome, Edge, Firefox) | done | `hostile_ext_events_are_ignored`, `hostile_native_message_too_large`, `manifest_json_shape` |
| Command line and scripting | done | `local_backend_runs_script_lines`, `cli_parses` |
| Tray icon (show, pause all, resume all, quit; close to tray) | partial (Windows only) | `tray_labels_exist_in_both_locales`; manual check on Windows |
| Installer and code signing | todo | owner decision; `cargo xtask dist` builds a folder |
| DRM / stream ripping | won't do | — |

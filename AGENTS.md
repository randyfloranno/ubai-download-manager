# AGENTS.md — UDM (Ubai Download Manager)

> Canonical rules for every human and AI agent working in this repository.
> Indonesian translation: `AGENTS.id.md`. If the two differ, **this file wins**.

---

## 1. Product

**UDM (Ubai Download Manager)** is a desktop download manager that speeds up and protects large downloads by splitting a file into several parallel connections (multi-segment, dynamic segmentation) and resuming safely after a pause, a dropped connection or a PC restart.

- **Feature equivalent of:** Internet Download Manager (IDM). IDM is a *behavioural reference only* (see §5.1, clean-room).
- **Who uses it:** everyday users, office staff, and IT people who download large files over unstable connections.
- **Core features:** multi-segment download with dynamic segmentation; pause/resume (also after disconnect or reboot); queues and scheduler (start time, stop time, shutdown/sleep after completion); per-download and global speed limits; automatic categories by file type (Documents, Video, Music, Programs, Archives) with target folders; browser integration (Chrome, Edge, Firefox) via extension + native messaging to capture links and directly accessible media; clipboard link capture; batch download from a URL list; simple site grabber; proxy and authentication (Basic/Digest, cookies handed over by the browser); checksum verification (MD5/SHA-256); history and search; completion notifications.
- **Protocols:** HTTP/1.1, HTTP/2, HTTPS, FTP, FTPS.
- **Platforms:** Windows, macOS, Linux (desktop). WebAssembly is used **only** for the browser extension.
- **Language:** Rust only (tokio for async, rustls for TLS). The single exception is the minimal JavaScript glue that the WebExtension API requires (§5.12).
- **UI language:** Indonesian (`id`) by default, English (`en`) as fallback. All UI text comes from the i18n catalogue.
- **Development machine:** Windows. Every command in this file must work in PowerShell; build tooling is written in Rust (`xtask`), not shell scripts.
- **Scope limit:** UDM only downloads content that is directly accessible to the user. No DRM circumvention, no decryption of licensed/encrypted media streams, no paywall or captcha bypass. Ever.

---

## 2. Start every session here

1. Read `plan/STATUS.md` — current milestone, task in progress, blockers, last green commit.
2. Read `plan/execution-plan.md` — milestones M0…M8, one-session tasks, acceptance criteria.
3. Read `plan/architecture.md` — crate map, layering, data flow.
4. Read every file in `plan/adr/` — accepted decisions are binding until superseded by a new ADR.
5. Follow the **autonomous protocol in `plan/execution-plan.md` §6**. In short:
   - pick the next unblocked task, mark it *in progress* in `plan/STATUS.md`;
   - write the failing test first, then the code;
   - run `cargo xtask ci` until green;
   - update `plan/STATUS.md`, docs (both languages) and `docs/en/parity-checklist-download-manager.md` if behaviour changed;
   - commit with an English message `<area>: <imperative summary>`;
   - stop and record the question in `plan/STATUS.md` under *Owner decisions needed* when a task needs an owner decision listed in §6 of the execution plan.

`plan/` is git-ignored working memory. It is written in English.

---

## 3. Never crash, never lose work

These rules apply to all non-test code in production crates (everything except `xtask`, `udm-testsrv` and `#[cfg(test)]` code).

1. **No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `unreachable!`, slice indexing that can panic, or `unsafe`.** Errors are returned as `Result<_, E>` with a typed error enum per crate. `unreachable!` is replaced by an error variant.
2. **Every production crate starts with this header** (`lib.rs` / `main.rs`):
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
   `udm-cli` may `allow(clippy::print_stdout)` in its output module only.
3. **All input is hostile:** URLs, HTTP/FTP responses, headers, `Content-Disposition`, redirects, HTML from the site grabber, native-messaging messages, control-port JSON, clipboard text, URL list files, state files on disk, and config files. Parse, validate, bound, then use.
4. **Hard limits live in `crates/udm-core/src/limits.rs`** as named constants (max redirects, header bytes and count, URL length, filename length, connections per download, segments, queue size, batch size, grabber depth/pages/page bytes, native message size, control message size, state file size, retry count, timeouts). No magic numbers elsewhere. Every limit has a `hostile_*` test proving it is enforced.
5. **Byte arithmetic is checked.** Offsets, lengths and ranges are `u64` and use `checked_*`/`saturating_*`; overflow is an error, never a wrap.
6. **Recursion is bounded or removed.** Redirect following, the site grabber and any parser are iterative or carry an explicit depth counter checked against `limits.rs`.
7. **Atomic saves.** Every file UDM writes for itself (resume state, queue, history, settings) is written to `*.tmp` in the same directory, `fsync`ed, then renamed over the target; the parent directory is `fsync`ed where the OS supports it. A torn or corrupt state file is detected (schema version + checksum) and reported, never silently used.
8. **Autosave.** Queue, history and settings autosave on change (debounced, ≤ 2 s). Resume checkpoints are written at least every 1 s or every 4 MiB per segment, on pause, on error and on shutdown.
9. **Panic guard.** Each download task, each command dispatch, the control server and each UI frame run behind `std::panic::catch_unwind` (or a `JoinHandle` panic check for tokio tasks). A panic in one download marks that download *Failed (internal error)* with its state preserved; it never takes down the app. `panic = "abort"` is forbidden in all profiles.
10. **Regression tests for hostile input are named `hostile_*`** (e.g. `hostile_content_disposition_path_traversal`). `cargo xtask ci` runs them as a separate, always-on group.

---

## 4. Download integrity and safety (domain production risks)

Every rule below is testable and has tests in CI. The local fault-injection server is `crates/udm-testsrv`; tests never touch the internet.

### 4.1 Never deliver a corrupt or incomplete file
1. **Part file + state.** While downloading, data goes to `<final-name>.udmpart` in the target folder; resume state goes to the data dir as `state/<download-id>.json` (atomic, §3.7). The final name appears **only** after finalisation succeeds.
2. **Durability order.** Segment data is flushed (`sync_data`) *before* the checkpoint that records it is committed. A checkpoint never claims bytes that are not on disk.
3. **Validators before resume.** Before resuming, UDM re-requests metadata and compares strong `ETag`, `Last-Modified` and total size against the saved state. Any mismatch → the part file is discarded and the download restarts from zero, with a visible notice (`download-restarted-remote-changed`). Weak ETags (`W/`) are not trusted for resume; fall back to `Last-Modified` + size, and restart if those are missing.
4. **Range responses are verified.** A `Range` request answered with `200` (not `206`) is never appended: the download falls back to one connection from offset 0. `Content-Range` must match the requested range and total size exactly; otherwise the segment fails and is retried. `Accept-Encoding: identity` is sent for file data.
5. **Coverage map.** Each download keeps a coverage map of completed byte ranges. Finalisation requires: no gaps, no overlaps, covered length == expected size, file length == expected size, and (if supplied) checksum match. Only then is `.udmpart` renamed to the final name.
6. **Unknown size.** Downloads without a known size use a single connection, cannot be split, and are marked "resume not guaranteed" in the UI.
7. **Determinism.** The segmentation planner and the dynamic re-splitting rule are pure functions of (size, connections, min segment size, coverage map). Property tests (proptest) check: segments cover `[0, size)` exactly once, never exceed `limits::MAX_SEGMENTS`, and are never smaller than `limits::MIN_SEGMENT_BYTES` except the tail.
8. **Golden fault-injection tests.** For each scenario the SHA-256 of the finished file must equal the fixture's SHA-256:
   connection drop mid-segment, server restart, app kill + restart (process-level test), ETag change between attempts, `Range` ignored (200), wrong `Content-Range`, `Content-Length` lies (short and long), slow server, server that sends more bytes than asked, HTTP/2 stream reset.

### 4.2 Malicious or broken servers
1. **Filename sanitisation** (from URL path, `Content-Disposition` `filename`/`filename*` RFC 6266/5987, or redirect target) is one function in `udm-proto` with `hostile_*` tests for: `..`, absolute paths, drive letters, `\` and `/`, NUL and control characters, Unicode bidi overrides (U+202E etc.), Windows reserved names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, with or without extension, any case), trailing dots/spaces, names longer than `limits::MAX_FILENAME_BYTES`, empty names, and dangerous double extensions shown in the UI (`invoice.pdf.exe`). The result is always a single path component inside the chosen folder; existing files are never overwritten without an explicit choice (auto-rename `name (1).ext`).
2. **Redirects** are capped at `limits::MAX_REDIRECTS`; loops are detected; an HTTPS→HTTP downgrade is refused unless the user allowed it for that download.
3. **Headers and bodies are bounded**: header bytes, header count, line length (FTP too), and grabber page size. A server streaming endless headers or a decompression bomb is cut off with an error.
4. **TLS verification is never disabled silently.** rustls with platform verification is the default. An "accept invalid certificate" exception can only be granted per host, by an explicit user action that shows the certificate details, is stored visibly in settings, is logged, and is never a global default or a CLI default. FTPS follows the same rule.
5. **Timeouts everywhere:** connect, TLS handshake, first byte, idle read. Values live in `limits.rs`.

### 4.3 Disk space
1. Before starting (and before resuming) UDM checks free space on the target volume against the remaining bytes + a safety margin (`limits::DISK_SAFETY_MARGIN_BYTES`). Not enough → the download does not start, with a clear message naming the volume and the shortfall.
2. Files with a known size are pre-allocated (real allocation, not just a sparse `set_len`, where the OS supports it).
3. A write error (`ENOSPC`, quota, removed drive) pauses the download with state intact and the message `error-disk-full` / `error-disk-write`; it never deletes or truncates data already written. Tests inject these errors through the writer trait.
4. Paths longer than the Windows legacy limit are handled (`\\?\` prefix through `std`/vetted crates); a path UDM cannot create is reported before downloading starts.

### 4.4 Credentials, cookies and privacy
1. Passwords, proxy credentials and stored cookies are kept only in the OS keychain (Windows Credential Manager, macOS Keychain, Secret Service on Linux) via `udm-secrets`. State files, settings, history, logs, screenshots, crash reports and control-port responses store **references**, never secrets.
2. Types holding secrets implement a redacting `Debug`/`Display` (`***`). A test greps logs and state files produced by the test suite for known secret fixtures and fails if any appear.
3. Cookies come **only** from the browser extension for the specific captured request, over native messaging. UDM never reads browser cookie databases or profile files.
4. No telemetry. Nothing leaves the machine except the downloads the user asked for.

### 4.5 Units and time
Sizes are `u64` bytes internally. The UI shows binary units with IEC labels (KiB, MiB, GiB) and speed per second, formatted by `udm-i18n` per locale (`1,5 MiB/s` in `id`, `1.5 MiB/s` in `en`). Timestamps are stored in UTC and shown in local time. Unit choice can only change through an ADR.

---

## 5. Non-negotiables

1. **Clean-room.** IDM may be observed as a black box (use the public product, read public web pages, compare behaviour). Never decompile, disassemble, patch, unpack or inspect its binaries, installers, registry data, extension code, file formats or network protocol; never copy its text, help files, dialog wording, icons, logos, sounds or screenshots into the repo. Its name appears only as a feature comparison. Unknown facts about IDM are written as *to verify* in `plan/reference/domain.md`, never invented.
2. **Assets and `ATTRIBUTION.md`.** Every non-code asset (icon, image, font, sound, MIME/category table, sample file, test fixture not generated by our code) has a row in `ATTRIBUTION.md` with path, author, source, licence and notes, in the same commit that adds it. No row → the asset does not merge. IDM's logo and icons and anything resembling them are forbidden.
3. **Content and legal scope.** Only directly accessible content. No DRM/Widevine/FairPlay/PlayReady handling, no decryption of encrypted HLS/DASH streams, no paywall, login-wall, captcha or rate-limit circumvention, no "YouTube ripper" features. Media capture is limited to direct file URLs the page already exposes. Proposals beyond that need an ADR approved by the owner.
4. **Bilingual UI via i18n catalogue.** All user-visible strings live in Fluent files `crates/udm-i18n/locales/id/udm.ftl` and `crates/udm-i18n/locales/en/udm.ftl`. Default locale `id`, fallback `en`. No display string literals in UI or engine code; errors carry message keys + arguments. `cargo xtask i18n` fails on missing keys, unused keys or keys present in one locale only. UI wording in `id` uses standard Indonesian (KBBI), e.g. *antrean*, *unduhan*, *jeda*, *lanjutkan*.
5. **Bilingual documentation.** User and developer docs live in `docs/en/` and `docs/id/` with identical file names. `cargo xtask docs` fails if a file exists in one language only. `ROADMAP.md` is bilingual in one file.
6. **Everything is a command.** Every user action is a `CommandSpec` registered in `udm-engine` (`id`, typed args, i18n label key, description key, whether it needs confirmation). Command ids are `<area>.<verb>` in lower snake case, e.g. `download.add`, `download.pause`, `download.resume`, `download.remove`, `download.verify`, `queue.start`, `queue.stop`, `schedule.set`, `limit.set_global`, `limit.set_download`, `category.assign`, `capture.clipboard_toggle`, `batch.import`, `grabber.start`, `settings.set`. The UI, CLI, control port, browser host and scheduler all go through the same dispatcher.
7. **Programmatic invocation without dialogs.** When a command receives all required args it runs without opening any dialog. Missing args → the GUI may open a dialog; headless callers (CLI, control port, native host) get a typed error instead. Confirmation prompts can be satisfied with `"confirm": true`.
8. **Layering enforced by `cargo xtask layers`.** A crate may depend only on crates in lower layers (table in `xtask/layers.toml`, mirrored in `plan/architecture.md`):

   | Layer | Crates | Notes |
   |---|---|---|
   | L0 | `udm-core` | ids, units, errors, `limits.rs`, `CommandSpec` types. **No dependencies.** Must build for `wasm32-unknown-unknown`. |
   | L1 | `udm-i18n`, `udm-proto`, `udm-store` | i18n (Fluent); pure protocol logic (URL, headers, `Content-Disposition`, filename sanitiser, range math, segmentation planner, link extraction); atomic state/queue/history storage. `udm-i18n` and `udm-proto` must build for wasm. |
   | L2 | `udm-net`, `udm-secrets`, `udm-sys` | tokio/rustls/HTTP/FTP/proxy/auth; OS keychain; disk space, pre-allocation, power actions, clipboard, notifications. |
   | L3 | `udm-engine` | downloads, segmentation runtime, queue, scheduler, speed limiter, categories, checksum, command registry + dispatcher. |
   | L4 | `udm-control` | JSON control protocol + server, `cmdline.input` parser. |
   | L5 | `udm-app` (bin `udm`, egui/eframe), `udm-cli` (bin `udm-cli`), `udm-native-host` (bin `udm-native-host`) | thin front-ends. |
   | Ext | `udm-ext` | browser extension core, `wasm32-unknown-unknown`; may depend on L0–L1 wasm-safe crates only. |
   | Dev | `udm-testsrv`, `xtask` | never a dependency of L0–L5 except as `dev-dependency` (and `udm-app`'s optional `dev-sample` feature). |

9. **Thin UI.** `udm-app` renders state and dispatches commands. No networking, file I/O, parsing or business rules in UI code. The UI is replaceable without touching L0–L4.
10. **Dependencies only from the allow-list** in `plan/adr/0001-dependencies.md` and `deny.toml`. Licences: MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, Unicode-3.0, CC0 and similar permissive licences. **GPL, LGPL, AGPL, SSPL, MPL-with-copyleft use and "non-commercial" licences are forbidden**, including copying code, porting algorithms line by line, or reading their source to reimplement it. Projects in this domain to avoid: aria2, GNU Wget, uGet, Xtreme Download Manager, JDownloader, Persepolis, KGet, FlashGot (verify each licence in `plan/reference/domain.md`; when unsure, treat as forbidden). Adding a dependency requires an ADR amendment in the same commit.
11. **One programming language; wasm must not break.** All source code in this repo is Rust, except the WebExtension glue in `extension/glue/` (§5.12). Third-party crates that build vendored C/asm (e.g. the rustls crypto provider) are allowed only if named in ADR 0001 with a reason. `cargo xtask ci` builds `udm-ext`, `udm-core`, `udm-proto` and `udm-i18n` for `wasm32-unknown-unknown`; a red wasm build blocks merge.
12. **Extension glue.** `extension/glue/*.js` only loads the wasm module and forwards WebExtension API calls (`runtime`, `contextMenus`, `downloads`, `tabs`, `cookies` for the captured request) to and from Rust. No parsing, filtering or decisions in JS. Total glue ≤ 300 lines, checked by `cargo xtask ci`. Manifest V3 for Chrome/Edge; Firefox manifest variant generated by `xtask`.
13. **Quality gates.** `cargo xtask ci` must pass before every commit. It runs: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, the `hostile_*` group, golden fault-injection tests, `cargo xtask layers`, `cargo xtask i18n`, `cargo xtask docs`, licence/allow-list check (`cargo deny check` when installed, otherwise the built-in `cargo metadata` checker), wasm builds, and the glue size check.

---

## 6. Domain glossary (short)

| English (code/docs) | Indonesian (UI) | Meaning |
|---|---|---|
| download | unduhan | One file being fetched, with its state. |
| segment | segmen | A byte range fetched by one connection. |
| connection | koneksi | One TCP/TLS or HTTP/2 stream used for a segment. |
| dynamic segmentation | segmentasi dinamis | When a segment finishes, the largest remaining one is split so idle connections keep working. |
| part file | berkas sementara (`.udmpart`) | Incomplete data file, renamed on finalisation. |
| resume state / checkpoint | status lanjutan / titik simpan | Persisted coverage map + validators. |
| validators | validator | `ETag`, `Last-Modified`, size; prove the remote file is unchanged. |
| pause / resume | jeda / lanjutkan | — |
| queue | antrean | Ordered list processed with a concurrency limit. |
| scheduler | penjadwal | Starts/stops queues at times; post-completion action. |
| speed limiter | pembatas kecepatan | Token bucket, global and per download. |
| category | kategori | File-type group with a target folder. |
| capture | tangkap | Taking a link from browser or clipboard. |
| native messaging host | host native messaging | `udm-native-host`, bridge between extension and UDM. |
| site grabber | site grabber (penyedot situs) | Crawls a page to bounded depth to list files. |
| batch download | unduh massal | Many URLs from a list or pattern. |
| checksum | checksum | MD5 (weak, comparison only) / SHA-256 hash of the file. |
| finalisation | finalisasi | Coverage + size + checksum check, then rename. |

---

## 7. Running and looking at the app

All commands work in PowerShell on Windows.

```powershell
# GUI with sample data and the agent control port
cargo run -p udm-app -- --sample --control 7878 --data-dir .udm-dev\agent-a

# Headless
cargo run -p udm-cli -- add http://127.0.0.1:8099/files/big.bin --dir .udm-dev\out --connections 8 --wait
cargo run -p udm-cli -- verify .udm-dev\out\big.bin --sha256 <hex>
cargo run -p udm-cli -- run --script .udm-dev\script.jsonl   # one control request per line

# Fault-injection test server on its own
cargo run -p udm-testsrv -- --port 8099
```

- `--sample` (feature `dev-sample`, on in dev builds, off in release packaging) starts an in-process `udm-testsrv` on a free port and seeds the queue with downloads covering every state (running, paused, failed, completed, scheduled, restarted-after-change). It never contacts the internet.
- `--control PORT` opens the control server on **127.0.0.1 only**. It is refused in release builds unless `UDM_ALLOW_CONTROL=1` is set. Protocol: newline-delimited JSON request/response.
- `--data-dir` isolates settings, state, queue and history. Default is the OS data dir.

Send requests with `cargo xtask ctl <PORT> '<json>'` (avoids netcat quirks on Windows):

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

Responses: `{"id":N,"ok":true,"result":{...}}` or `{"id":N,"ok":false,"error":{"key":"error-...","args":{...}}}`. Error keys are i18n keys. Secrets never appear in responses.

**Look at the UI after every UI change:** take screenshots in `id` and `en` (`ui.set_locale` then `ui.screenshot`) and inspect them before committing.

**Parallel agents:**
- one git worktree per agent; never two agents in one working tree;
- each agent uses its own `--control` port, its own `--data-dir .udm-dev\<agent>`, and lets `udm-testsrv` pick a free port (`--port 0`);
- claim a task in `plan/STATUS.md` before starting; never work on a task claimed by another agent;
- only one agent edits `xtask/layers.toml`, `deny.toml`, ADRs or the Fluent catalogues at a time; record the lock in `plan/STATUS.md`.

---

## 8. Roadmap

- `ROADMAP.md` — bilingual milestone overview for humans (M0…M8).
- `docs/en/gaps.md` / `docs/id/gaps.md` — known gaps and deliberate non-features (e.g. no DRM, no stream ripping).
- `docs/en/parity-checklist-download-manager.md` / `docs/id/parity-checklist-download-manager.md` — feature-by-feature comparison with the reference product, observed behaviour only, each row with status (`todo` / `partial` / `done` / `won't do`) and the test that proves it.

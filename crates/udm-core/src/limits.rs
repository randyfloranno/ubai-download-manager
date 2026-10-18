//! Hard limits. Every bound UDM enforces on hostile input lives here.
//! No magic numbers elsewhere: other crates reference these constants.
//! Each constant has a `hostile_*` test (checked by `cargo xtask limits`).

// ---- URLs, headers, protocol framing ----

/// Maximum URL length in bytes.
pub const MAX_URL_BYTES: usize = 8_192;
/// Maximum number of redirects followed for one request.
pub const MAX_REDIRECTS: u32 = 10;
/// Maximum total bytes of a response header block.
pub const MAX_HEADER_BYTES: usize = 65_536;
/// Maximum number of response headers.
pub const MAX_HEADER_COUNT: usize = 128;
/// Maximum length of one header value we parse (Content-Disposition, ETag, ...).
pub const MAX_HEADER_VALUE_BYTES: usize = 8_192;
/// Maximum length of one FTP control-channel line.
pub const MAX_FTP_LINE_BYTES: usize = 4_096;
/// Maximum number of lines in one multi-line FTP reply.
pub const MAX_FTP_REPLY_LINES: usize = 256;

// ---- Files and paths ----

/// Maximum file name length in UTF-8 bytes (leaves room for " (999)" and ".udmpart").
pub const MAX_FILENAME_BYTES: usize = 200;
/// Maximum full path length in UTF-8 bytes we accept for a target path.
pub const MAX_PATH_BYTES: usize = 32_000;
/// Maximum number of auto-rename attempts (`name (1).ext` ... `name (N).ext`).
pub const MAX_AUTO_RENAME: u32 = 999;
/// Free-space safety margin required on top of the remaining bytes (64 MiB).
pub const DISK_SAFETY_MARGIN_BYTES: u64 = 67_108_864;

// ---- Segmentation ----

/// Default connections per download.
pub const DEFAULT_CONNECTIONS: u32 = 8;
/// Maximum connections per download.
pub const MAX_CONNECTIONS_PER_DOWNLOAD: u32 = 32;
/// Maximum segments a download may ever be split into.
pub const MAX_SEGMENTS: usize = 1_024;
/// Minimum segment size except the tail (256 KiB).
pub const MIN_SEGMENT_BYTES: u64 = 262_144;
/// Split points are aligned to this many bytes (4 KiB).
pub const SEGMENT_ALIGN_BYTES: u64 = 4_096;
/// Maximum supported file size (16 TiB).
pub const MAX_FILE_SIZE_BYTES: u64 = 17_592_186_044_416;

// ---- Retry and timeouts ----

/// Maximum retries per segment before the download fails.
pub const MAX_SEGMENT_RETRIES: u32 = 8;
/// Maximum consecutive retries after a server protocol violation (wrong
/// `Content-Range`, contradicting lengths, extra bytes) without progress.
pub const MAX_PROTOCOL_RETRIES: u32 = 2;
/// Maximum transfer attempts of one connection worker in one run, progress or
/// not; stops a server that always sends a little and then breaks.
pub const MAX_TRANSFER_ATTEMPTS: u32 = 64;
/// Base retry back-off in milliseconds (doubled per attempt).
pub const RETRY_BACKOFF_BASE_MS: u64 = 500;
/// Maximum retry back-off in milliseconds.
pub const RETRY_BACKOFF_MAX_MS: u64 = 30_000;
/// TCP connect timeout.
pub const CONNECT_TIMEOUT_MS: u64 = 15_000;
/// TLS handshake timeout.
pub const TLS_HANDSHAKE_TIMEOUT_MS: u64 = 15_000;
/// Time allowed between sending a request and receiving the response head.
pub const FIRST_BYTE_TIMEOUT_MS: u64 = 30_000;
/// Maximum silence while reading a body.
pub const IDLE_READ_TIMEOUT_MS: u64 = 30_000;

// ---- Checkpoints and autosave ----

/// Resume checkpoint interval.
pub const CHECKPOINT_INTERVAL_MS: u64 = 1_000;
/// Resume checkpoint after this many new bytes per segment (4 MiB).
pub const CHECKPOINT_BYTES: u64 = 4_194_304;
/// Autosave debounce for queue/history/settings (≤ 2 s).
pub const AUTOSAVE_DEBOUNCE_MS: u64 = 1_000;
/// Maximum size of a state/settings/queue file read from disk (16 MiB).
pub const MAX_STATE_FILE_BYTES: u64 = 16_777_216;
/// Maximum size of one history line.
pub const MAX_HISTORY_LINE_BYTES: usize = 65_536;
/// Maximum history records kept (oldest dropped on compaction).
pub const MAX_HISTORY_RECORDS: usize = 100_000;

// ---- Engine ----

/// Maximum downloads held by the engine (all queues).
pub const MAX_QUEUE_SIZE: usize = 10_000;
/// Maximum number of queues.
pub const MAX_QUEUES: usize = 64;
/// Maximum simultaneous downloads per queue.
pub const MAX_CONCURRENT_DOWNLOADS: u32 = 32;
/// Maximum URLs accepted in one batch import.
pub const MAX_BATCH_SIZE: usize = 10_000;
/// Maximum URL list file size (8 MiB).
pub const MAX_URL_LIST_BYTES: usize = 8_388_608;
/// Maximum speed limit value (bytes per second, 10 GiB/s); 0 means unlimited.
pub const MAX_SPEED_LIMIT_BYTES_PER_SEC: u64 = 10_737_418_240;
/// Maximum categories.
pub const MAX_CATEGORIES: usize = 64;
/// Maximum extensions per category.
pub const MAX_CATEGORY_EXTENSIONS: usize = 256;

// ---- Site grabber ----

/// Maximum crawl depth.
pub const MAX_GRABBER_DEPTH: u32 = 5;
/// Maximum pages fetched per crawl.
pub const MAX_GRABBER_PAGES: usize = 500;
/// Maximum bytes of one page (4 MiB).
pub const MAX_GRABBER_PAGE_BYTES: usize = 4_194_304;
/// Maximum links extracted from one page.
pub const MAX_LINKS_PER_PAGE: usize = 10_000;

// ---- IPC ----

/// Maximum native-messaging message size, both directions (1 MiB).
pub const MAX_NATIVE_MESSAGE_BYTES: usize = 1_048_576;
/// Maximum control-port message (one JSON line) size (1 MiB).
pub const MAX_CONTROL_MESSAGE_BYTES: usize = 1_048_576;
/// Maximum simultaneous control connections.
pub const MAX_CONTROL_CONNECTIONS: usize = 16;
/// Seconds the control port waits for the GUI to answer a `ui.*` request.
pub const UI_REPLY_TIMEOUT_SECS: u64 = 10;
/// Maximum pending `ui.*` requests waiting for the GUI.
pub const MAX_PENDING_UI_REQUESTS: usize = 32;
/// Maximum command-line text accepted by `cmdline.input`.
pub const MAX_CMDLINE_BYTES: usize = 16_384;
/// Maximum command-line tokens.
pub const MAX_CMDLINE_TOKENS: usize = 256;
/// Maximum size of a `udm-cli run --script` file (8 MiB).
pub const MAX_SCRIPT_BYTES: u64 = 8_388_608;
/// Maximum clipboard text inspected for links.
pub const MAX_CLIPBOARD_BYTES: usize = 65_536;
/// Maximum length of a secret reference (keychain entry name) in bytes.
pub const MAX_SECRET_REF_BYTES: usize = 512;
/// Maximum cookie header bytes passed from the browser for one request.
pub const MAX_COOKIE_BYTES: usize = 16_384;
/// Maximum number of arguments in one command invocation.
pub const MAX_COMMAND_ARGS: usize = 64;
/// Maximum length of one string argument.
pub const MAX_ARG_STRING_BYTES: usize = 65_536;
/// Maximum number of items in one list argument.
pub const MAX_ARG_LIST_ITEMS: usize = 10_000;
/// Largest application icon edge rendered by `logo` (Windows ICO maximum).
pub const MAX_ICON_PX: u16 = 256;

//! The runtime of one download: probe, resume validation, part file,
//! parallel segment workers, checkpoints and finalisation (ADR 0003).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tokio::task::JoinSet;
use udm_core::{ByteRange, CoverageMap, DownloadId, Message, PauseReason, limits};
use udm_net::{Client, FetchBody, NetError, ProbeInfo, Request};
use udm_proto::{ChecksumSpec, EntityTag};
use udm_store::{DataDir, load_envelope, save_envelope};
use udm_sys::FileWriter;

use crate::board::Board;
use crate::checksum::hash_file;
use crate::error::{EngineError, EngineResult};
use crate::limiter::Limiter;
use crate::model::ResumeState;
use crate::platform::Platform;

/// Envelope kind of resume checkpoints.
pub const RESUME_KIND: &str = "resume";
/// Schema version of resume checkpoints.
pub const RESUME_SCHEMA: u32 = 1;
/// Suffix of part files.
pub const PART_SUFFIX: &str = ".udmpart";

/// Why a running download is asked to stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopKind {
    /// Pause with a reason.
    Pause(PauseReason),
    /// Stop and put back into the queue (schedule stop time).
    Requeue,
    /// Remove the download (optionally deleting files).
    Remove {
        /// Delete the part file too.
        delete_file: bool,
    },
}

/// Phase reported to the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Talking to the server.
    Probing,
    /// Transferring.
    Downloading,
    /// Verifying and renaming.
    Finalising,
}

/// Live, shared state of a running download.
#[derive(Debug)]
pub struct Live {
    /// Segment board (present once the part file is open).
    pub board: Mutex<Option<Board>>,
    /// Stop signal.
    pub stop: watch::Sender<Option<StopKind>>,
    /// Per-download limiter.
    pub limiter: Limiter,
    /// Desired connections.
    pub connections: AtomicU32,
    /// Workers currently transferring.
    pub active: AtomicU32,
    /// Smoothed speed in bytes per second.
    pub speed: AtomicU64,
    /// Current phase.
    pub phase: Mutex<Phase>,
    /// Facts learned while running (size, name, ...).
    pub facts: Mutex<Facts>,
}

/// Facts the engine copies into the record while the task runs.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// Expected size.
    pub size: Option<u64>,
    /// Final file name.
    pub file_name: Option<String>,
    /// Target folder.
    pub dir: Option<PathBuf>,
    /// Category chosen after probing.
    pub category: Option<String>,
    /// Ranges supported.
    pub resumable: Option<bool>,
    /// Notice to show (e.g. restarted because the remote changed).
    pub notice: Option<Message>,
}

impl Live {
    /// Creates live state.
    pub fn new(connections: u32, speed_limit: u64) -> Self {
        let (stop, _) = watch::channel(None);
        Self {
            board: Mutex::new(None),
            stop,
            limiter: Limiter::new(speed_limit),
            connections: AtomicU32::new(connections),
            active: AtomicU32::new(0),
            speed: AtomicU64::new(0),
            phase: Mutex::new(Phase::Probing),
            facts: Mutex::new(Facts::default()),
        }
    }

    /// Requests a stop. The first request wins.
    pub fn request_stop(&self, kind: StopKind) {
        self.stop.send_if_modified(|cur| {
            if cur.is_none() {
                *cur = Some(kind);
                true
            } else {
                false
            }
        });
    }

    fn stop_kind(&self) -> Option<StopKind> {
        *self.stop.borrow()
    }

    fn set_phase(&self, p: Phase) {
        *lock(&self.phase) = p;
    }

    /// Current phase.
    pub fn phase(&self) -> Phase {
        *lock(&self.phase)
    }

    fn facts(&self) -> MutexGuard<'_, Facts> {
        lock(&self.facts)
    }
}

/// Locks a mutex, recovering from poisoning (a panicked holder already
/// marked its download failed; the data stays usable).
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Everything a task needs, captured when it starts.
#[derive(Clone)]
pub struct TaskInput {
    /// Id.
    pub id: DownloadId,
    /// Request (URL, cookie, credentials, referer).
    pub request: Request,
    /// Target folder given by the user, if any.
    pub dir: Option<PathBuf>,
    /// File name chosen earlier (record) or requested by the user.
    pub file_name: Option<String>,
    /// File name was chosen by the user (do not replace with server name).
    pub name_from_user: bool,
    /// Category; `None` = classify after probing.
    pub category: Option<String>,
    /// Expected checksum.
    pub checksum: Option<ChecksumSpec>,
    /// Network client.
    pub client: Arc<Client>,
    /// Global limiter.
    pub global: Arc<Limiter>,
    /// OS seams.
    pub platform: Arc<dyn Platform>,
    /// Data dir.
    pub data_dir: DataDir,
    /// Resolves `(category or None, file name)` to `(category, folder)`.
    pub folders: FolderFn,
}

/// Resolves `(category or None, file name)` to `(category, folder)`.
pub type FolderFn = Arc<dyn Fn(Option<&str>, &str) -> (String, PathBuf) + Send + Sync>;

/// How a task ended.
#[derive(Debug)]
pub enum Outcome {
    /// Finished and renamed.
    Completed {
        /// Final path.
        path: PathBuf,
        /// Size.
        size: u64,
        /// SHA-256 when a checksum was verified with it.
        sha256: Option<String>,
    },
    /// Stopped on request.
    Stopped(StopKind),
    /// Paused by the engine (disk full, write error).
    Paused(PauseReason, Message),
    /// Failed.
    Failed(Message),
}

/// Runs a download to its end.
pub async fn run(input: TaskInput, live: Arc<Live>) -> Outcome {
    match run_inner(&input, &live).await {
        Ok(o) => o,
        Err(Stop::Requested(k)) => Outcome::Stopped(k),
        Err(Stop::Paused(r, m)) => Outcome::Paused(r, m),
        Err(Stop::Error(e)) => classify_failure(e),
    }
}

/// Converts an error into a pause (disk problems) or a failure.
fn classify_failure(e: EngineError) -> Outcome {
    let msg = e.to_message();
    match is_disk_error(&msg) {
        Some(reason) => Outcome::Paused(reason, msg),
        None => Outcome::Failed(msg),
    }
}

enum Stop {
    Requested(StopKind),
    Paused(PauseReason, Message),
    Error(EngineError),
}

impl<E: Into<EngineError>> From<E> for Stop {
    fn from(e: E) -> Self {
        Stop::Error(e.into())
    }
}

fn now_check(live: &Live) -> Result<(), Stop> {
    match live.stop_kind() {
        Some(k) => Err(Stop::Requested(k)),
        None => Ok(()),
    }
}

/// Waits `d` or until a stop is requested.
async fn sleep_or_stop(live: &Live, d: Duration) -> Result<(), Stop> {
    let mut rx = live.stop.subscribe();
    if let Some(k) = *rx.borrow() {
        return Err(Stop::Requested(k));
    }
    tokio::select! {
        () = tokio::time::sleep(d) => Ok(()),
        _ = rx.changed() => now_check(live),
    }
}

fn backoff(attempt: u32) -> Duration {
    let factor = 1u64.checked_shl(attempt.min(16)).unwrap_or(u64::MAX);
    Duration::from_millis(
        limits::RETRY_BACKOFF_BASE_MS
            .saturating_mul(factor)
            .min(limits::RETRY_BACKOFF_MAX_MS),
    )
}

async fn probe_with_retry(input: &TaskInput, live: &Live) -> Result<ProbeInfo, Stop> {
    let mut attempt = 0u32;
    loop {
        now_check(live)?;
        let mut rx = live.stop.subscribe();
        let res = tokio::select! {
            r = input.client.probe(&input.request) => r,
            _ = rx.changed() => { now_check(live)?; continue; }
        };
        match res {
            Ok(p) => return Ok(p),
            Err(e) if e.is_retryable() && attempt < limits::MAX_SEGMENT_RETRIES => {
                tracing::debug!(id = %input.id, attempt, "probe retry: {}", e.to_message());
                sleep_or_stop(live, backoff(attempt)).await?;
                attempt = attempt.saturating_add(1);
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// True when the saved validators still describe the remote file.
pub fn validators_match(saved: &ResumeState, probe: &ProbeInfo) -> bool {
    if !probe.accepts_ranges || !saved.resumable {
        return false;
    }
    match (saved.size, probe.size) {
        (Some(a), Some(b)) if a == b => {}
        _ => return false,
    }
    let saved_tag = saved.etag.as_deref().and_then(EntityTag::parse);
    match (saved_tag, probe.etag.as_ref()) {
        (Some(a), Some(b)) if !a.weak && !b.weak => return a.strong_eq(b),
        // A strong tag that disappeared or changed strength: cannot prove identity.
        (Some(a), _) if !a.weak => return false,
        _ => {}
    }
    // Weak or missing ETag: Last-Modified + size must match.
    match (&saved.last_modified, &probe.last_modified) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Picks a free final name in `dir` (no existing file and no foreign part file).
fn choose_free_name(dir: &Path, name: &str) -> Result<String, Stop> {
    for n in 0..=limits::MAX_AUTO_RENAME {
        let candidate = if n == 0 {
            name.to_owned()
        } else {
            udm_proto::auto_rename(name, n)
        };
        let final_path = dir.join(&candidate);
        let part_path = dir.join(format!("{candidate}{PART_SUFFIX}"));
        if !final_path.exists() && !part_path.exists() {
            return Ok(candidate);
        }
    }
    Err(Stop::Error(EngineError::Msg(
        Message::new("error-engine-no-free-name").with_str("name", name),
    )))
}

fn load_resume(input: &TaskInput) -> (Option<ResumeState>, Option<Message>) {
    let path = input.data_dir.state_file(input.id);
    match load_envelope::<ResumeState>(&path, RESUME_KIND, RESUME_SCHEMA) {
        Ok(Some((_, s))) if s.id == input.id.to_string() => (Some(s), None),
        Ok(Some(_)) => (None, Some(Message::new("notice-resume-state-corrupt"))),
        Ok(None) => (None, None),
        Err(e) => {
            tracing::warn!(id = %input.id, "resume state unusable: {}", e.to_message());
            (None, Some(Message::new("notice-resume-state-corrupt")))
        }
    }
}

/// Part file recorded in the saved resume state, if any. Only paths that end
/// in [`PART_SUFFIX`] are returned, so a tampered state file cannot point
/// removal at an arbitrary file.
pub(crate) fn saved_part_path(data_dir: &DataDir, id: DownloadId) -> Option<PathBuf> {
    let (_, s) = load_envelope::<ResumeState>(&data_dir.state_file(id), RESUME_KIND, RESUME_SCHEMA)
        .ok()
        .flatten()?;
    let is_part = s
        .part_path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(PART_SUFFIX) && n.len() > PART_SUFFIX.len());
    (s.id == id.to_string() && is_part).then_some(s.part_path)
}

fn save_checkpoint(
    data_dir: &DataDir,
    id: DownloadId,
    base: &ResumeState,
    coverage: &CoverageMap,
) -> EngineResult<()> {
    let mut st = base.clone();
    st.coverage = coverage
        .ranges()
        .iter()
        .map(|r| (r.start(), r.end()))
        .collect();
    save_envelope(&data_dir.state_file(id), RESUME_KIND, RESUME_SCHEMA, &st)?;
    Ok(())
}

/// Durability order: snapshot coverage, then `sync_data`, then commit the
/// checkpoint that records it. Runs on the blocking pool.
async fn checkpoint(
    live: &Arc<Live>,
    writer: &Arc<dyn FileWriter>,
    data_dir: &DataDir,
    id: DownloadId,
    base: &ResumeState,
) -> EngineResult<()> {
    let part = base.part_path.clone();
    let snapshot = match lock(&live.board).as_ref() {
        Some(b) => b.snapshot(),
        None => return Ok(()),
    };
    let writer = Arc::clone(writer);
    let data_dir = data_dir.clone();
    let base = base.clone();
    let res = tokio::task::spawn_blocking(move || -> EngineResult<()> {
        writer.sync_data().map_err(|e| write_error(&part, &e))?;
        save_checkpoint(&data_dir, id, &base, &snapshot)
    })
    .await;
    match res {
        Ok(r) => r,
        Err(_) => Err(EngineError::key("error-engine-internal")),
    }
}

fn write_error(path: &Path, e: &std::io::Error) -> EngineError {
    EngineError::Sys(udm_sys::SysError::from_write_error(path, e))
}

fn is_disk_error(m: &Message) -> Option<PauseReason> {
    match m.key.as_str() {
        "error-disk-full" | "error-sys-not-enough-space" => Some(PauseReason::DiskFull),
        "error-disk-write" => Some(PauseReason::DiskWrite),
        _ => None,
    }
}

async fn run_inner(input: &TaskInput, live: &Arc<Live>) -> Result<Outcome, Stop> {
    live.set_phase(Phase::Probing);
    let probe = probe_with_retry(input, live).await?;
    let (mut saved, corrupt_notice) = load_resume(input);
    if let Some(n) = corrupt_notice {
        live.facts().notice = Some(n);
    }

    let resumable = probe.accepts_ranges && probe.size.is_some();

    // Validators before resume (ADR 0003 §3).
    if let Some(s) = &saved
        && !validators_match(s, &probe)
    {
        let part = s.part_path.clone();
        tracing::info!(id = %input.id, "remote changed, restarting from zero");
        let _ = std::fs::remove_file(&part);
        live.facts().notice = Some(Message::new("download-restarted-remote-changed"));
        // Keep the chosen name so the user sees the same file.
        saved = Some(ResumeState {
            coverage: Vec::new(),
            size: probe.size,
            etag: probe.etag.as_ref().map(EntityTag::to_header),
            last_modified: probe.last_modified.clone(),
            final_url: probe.final_url.to_string(),
            resumable,
            ..s.clone()
        });
    }

    let base = match saved {
        Some(s) if s.part_path.exists() || s.coverage.is_empty() => s,
        Some(s) => {
            // Part file vanished: start over with the same name.
            ResumeState {
                coverage: Vec::new(),
                ..s
            }
        }
        None => {
            let raw = if input.name_from_user {
                input.file_name.clone()
            } else {
                probe
                    .filename
                    .clone()
                    .or_else(|| udm_proto::filename_from_url(&probe.final_url))
                    .or_else(|| input.file_name.clone())
            };
            let name = udm_proto::sanitize_filename(raw.as_deref().unwrap_or(""));
            let (category, default_dir) = (input.folders)(input.category.as_deref(), &name);
            let dir = input.dir.clone().unwrap_or(default_dir);
            std::fs::create_dir_all(udm_sys::long_path(&dir)).map_err(|e| {
                EngineError::Msg(
                    Message::new("error-engine-create-dir")
                        .with_str("path", dir.display().to_string())
                        .with_str("detail", e.kind().to_string()),
                )
            })?;
            let name = choose_free_name(&dir, &name)?;
            {
                let mut f = live.facts();
                f.category = Some(category);
                f.dir = Some(dir.clone());
            }
            ResumeState {
                id: input.id.to_string(),
                final_url: probe.final_url.to_string(),
                size: probe.size,
                etag: probe.etag.as_ref().map(EntityTag::to_header),
                last_modified: probe.last_modified.clone(),
                part_path: dir.join(format!("{name}{PART_SUFFIX}")),
                file_name: name,
                coverage: Vec::new(),
                resumable,
            }
        }
    };
    let dir = base
        .part_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    {
        let mut f = live.facts();
        f.size = probe.size;
        f.file_name = Some(base.file_name.clone());
        f.resumable = Some(resumable);
        f.dir = Some(dir.clone());
    }

    let ranges: Vec<ByteRange> = base
        .coverage
        .iter()
        .filter_map(|(a, b)| ByteRange::new(*a, *b))
        .collect();
    let coverage = CoverageMap::from_ranges(&ranges).unwrap_or_default();

    // Free space before (re)starting.
    if let Some(size) = probe.size {
        let remaining = size.saturating_sub(coverage.covered_len());
        input.platform.ensure_space(&dir, remaining)?;
    }

    let fresh = coverage.covered_len() == 0;
    let writer =
        input
            .platform
            .open_part(&base.part_path, probe.size, fresh && probe.size.is_some())?;

    let connections = live.connections.load(Ordering::Relaxed);
    *lock(&live.board) = Some(Board::new(probe.size, connections, resumable, coverage));
    checkpoint(live, &writer, &input.data_dir, input.id, &base).await?;
    live.set_phase(Phase::Downloading);

    let validator = probe.etag.clone().filter(|t| !t.weak);
    let mut request = input.request.clone();
    request.url = probe.final_url.clone();
    let ctx = Arc::new(WorkerCtx {
        part_path: base.part_path.clone(),
        client: Arc::clone(&input.client),
        request,
        writer: Arc::clone(&writer),
        live: Arc::clone(live),
        global: Arc::clone(&input.global),
        validator,
        resumable,
    });

    supervise(&ctx, input, &base).await?;

    // ---- Finalisation ----
    live.set_phase(Phase::Finalising);
    let size = {
        let guard = lock(&live.board);
        let Some(board) = guard.as_ref() else {
            return Err(EngineError::key("error-engine-internal").into());
        };
        if !board.is_complete() {
            return Err(EngineError::key("error-engine-incomplete").into());
        }
        board.size().unwrap_or(0)
    };
    {
        let w = Arc::clone(&writer);
        let synced = tokio::task::spawn_blocking(move || w.sync_data().map(|()| w.len())).await;
        match synced {
            Ok(Ok(Ok(len))) if len == size => {}
            Ok(Ok(Ok(len))) => {
                return Err(EngineError::Msg(
                    Message::new("error-engine-size-mismatch")
                        .with_bytes("expected", size)
                        .with_bytes("actual", len),
                )
                .into());
            }
            Ok(Ok(Err(e)) | Err(e)) => return Err(write_error(&base.part_path, &e).into()),
            Err(_) => return Err(EngineError::key("error-engine-internal").into()),
        }
    }
    drop(ctx);
    drop(writer);

    let mut sha256 = None;
    if let Some(spec) = &input.checksum {
        let path = base.part_path.clone();
        let spec2 = spec.clone();
        let res = tokio::task::spawn_blocking(move || hash_file(&path, spec2.algo)).await;
        let digest = match res {
            Ok(r) => r?,
            Err(_) => return Err(EngineError::key("error-engine-internal").into()),
        };
        if digest != spec.expected {
            return Err(EngineError::Msg(
                Message::new("error-engine-checksum-mismatch")
                    .with_str("expected", udm_proto::to_hex(&spec.expected))
                    .with_str("actual", udm_proto::to_hex(&digest)),
            )
            .into());
        }
        if matches!(spec.algo, udm_proto::ChecksumAlgo::Sha256) {
            sha256 = Some(udm_proto::to_hex(&digest));
        }
    }

    // Rename; never overwrite a file that appeared meanwhile.
    let mut final_name = base.file_name.clone();
    let mut attempt = 0u32;
    let final_path = loop {
        let target = dir.join(&final_name);
        match input.platform.rename_no_overwrite(&base.part_path, &target) {
            Ok(()) => break target,
            Err(udm_sys::SysError::AlreadyExists { .. }) if attempt < limits::MAX_AUTO_RENAME => {
                attempt = attempt.saturating_add(1);
                final_name = udm_proto::auto_rename(&base.file_name, attempt);
            }
            Err(e) => return Err(e.into()),
        }
    };
    live.facts().file_name = Some(final_name);
    let _ = std::fs::remove_file(input.data_dir.state_file(input.id));
    Ok(Outcome::Completed {
        path: final_path,
        size,
        sha256,
    })
}

struct WorkerCtx {
    part_path: PathBuf,
    client: Arc<Client>,
    request: Request,
    writer: Arc<dyn FileWriter>,
    live: Arc<Live>,
    global: Arc<Limiter>,
    validator: Option<EntityTag>,
    resumable: bool,
}

/// Runs workers, checkpoints and speed sampling until the board is done,
/// a stop is requested or a fatal error occurs.
async fn supervise(
    ctx: &Arc<WorkerCtx>,
    input: &TaskInput,
    base: &ResumeState,
) -> Result<(), Stop> {
    let live = &ctx.live;
    let mut workers: JoinSet<Result<(), EngineError>> = JoinSet::new();
    let mut spawned_total = 0u32;
    let mut last_ckpt = Instant::now();
    let mut last_ckpt_bytes = downloaded(live);
    let mut last_sample = Instant::now();
    let mut last_sample_bytes = last_ckpt_bytes;
    let mut speed_ewma = 0f64;
    let mut fatal: Option<EngineError> = None;
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut stop_rx = live.stop.subscribe();

    loop {
        // Spawn workers up to the desired count while work exists.
        let desired = live
            .connections
            .load(Ordering::Relaxed)
            .clamp(1, limits::MAX_CONNECTIONS_PER_DOWNLOAD);
        let single = lock(&live.board).as_ref().is_some_and(Board::is_single);
        let target = if single { 1 } else { desired };
        let has_work = lock(&live.board).as_ref().is_some_and(|b| !b.is_idle());
        while fatal.is_none()
            && has_work
            && u32::try_from(workers.len()).unwrap_or(u32::MAX) < target
            && live.stop_kind().is_none()
        {
            let c = Arc::clone(ctx);
            workers.spawn(async move { worker(c).await });
            spawned_total = spawned_total.saturating_add(1);
        }

        if workers.is_empty() {
            break;
        }

        tokio::select! {
            joined = workers.join_next() => {
                match joined {
                    Some(Ok(Ok(()))) | None => {}
                    Some(Ok(Err(e))) => {
                        if fatal.is_none() {
                            fatal = Some(e);
                            // Stop the other workers: one fatal error ends the attempt.
                            live.request_stop(StopKind::Requeue);
                        }
                    }
                    Some(Err(join)) => {
                        tracing::error!(id = %input.id, "worker panicked: {join}");
                        if fatal.is_none() {
                            fatal = Some(EngineError::key("error-engine-internal"));
                            live.request_stop(StopKind::Requeue);
                        }
                    }
                }
            }
            _ = tick.tick() => {}
            _ = stop_rx.changed() => {}
        }

        let now = Instant::now();
        let bytes = downloaded(live);
        let dt = now.saturating_duration_since(last_sample).as_secs_f64();
        if dt >= 0.5 {
            let inst = (bytes.saturating_sub(last_sample_bytes)) as f64 / dt;
            speed_ewma = if speed_ewma == 0.0 {
                inst
            } else {
                speed_ewma * 0.7 + inst * 0.3
            };
            live.speed.store(f64_to_u64(speed_ewma), Ordering::Relaxed);
            last_sample = now;
            last_sample_bytes = bytes;
        }
        let since = now.saturating_duration_since(last_ckpt);
        let per_seg_bytes = limits::CHECKPOINT_BYTES
            .saturating_mul(u64::from(live.active.load(Ordering::Relaxed).max(1)));
        if since >= Duration::from_millis(limits::CHECKPOINT_INTERVAL_MS)
            || bytes.saturating_sub(last_ckpt_bytes) >= per_seg_bytes
        {
            if let Err(e) = checkpoint(live, &ctx.writer, &input.data_dir, input.id, base).await
                && fatal.is_none()
            {
                fatal = Some(e);
                live.request_stop(StopKind::Requeue);
            }
            last_ckpt = now;
            last_ckpt_bytes = bytes;
        }
    }
    live.speed.store(0, Ordering::Relaxed);
    live.active.store(0, Ordering::Relaxed);

    // Final checkpoint on pause, error and completion.
    let ck = checkpoint(live, &ctx.writer, &input.data_dir, input.id, base).await;
    if let Some(e) = fatal {
        let msg = e.to_message();
        if let Some(reason) = is_disk_error(&msg) {
            return Err(Stop::Paused(reason, msg));
        }
        return Err(Stop::Error(e));
    }
    if let Some(k) = live.stop_kind() {
        return Err(Stop::Requested(k));
    }
    ck?;
    Ok(())
}

fn f64_to_u64(v: f64) -> u64 {
    if v.is_finite() && v > 0.0 {
        // Bounded: speeds are far below 2^53.
        format!("{v:.0}").parse().unwrap_or(0)
    } else {
        0
    }
}

fn downloaded(live: &Live) -> u64 {
    lock(&live.board).as_ref().map_or(0, Board::downloaded)
}

/// One connection: claims ranges and streams them into the part file.
async fn worker(ctx: Arc<WorkerCtx>) -> Result<(), EngineError> {
    let live = &ctx.live;
    let mut failures = 0u32;
    let mut protocol_failures = 0u32;
    let mut attempts = 0u32;
    loop {
        if live.stop_kind().is_some() {
            return Ok(());
        }
        let Some(claim) = lock(&live.board).as_mut().and_then(Board::claim) else {
            return Ok(());
        };
        attempts = attempts.saturating_add(1);
        live.active.fetch_add(1, Ordering::Relaxed);
        let res = transfer(&ctx, claim.slot).await;
        live.active.fetch_sub(1, Ordering::Relaxed);
        match res {
            Ok(progress) => {
                if progress {
                    failures = 0;
                }
            }
            Err(TransferError::Stopped) => {
                release(live, claim.slot);
                return Ok(());
            }
            Err(TransferError::Fatal(e)) => {
                release(live, claim.slot);
                return Err(e);
            }
            Err(TransferError::Retry(e, progress)) => {
                release(live, claim.slot);
                if progress {
                    failures = 0;
                }
                failures = failures.saturating_add(1);
                if is_protocol_violation(&e) && !progress {
                    protocol_failures = protocol_failures.saturating_add(1);
                } else {
                    protocol_failures = 0;
                }
                if retries_exhausted(failures, protocol_failures, attempts) {
                    return Err(e.into());
                }
                tracing::debug!("segment retry {failures}: {}", e.to_message());
                if sleep_or_stop(live, backoff(failures.saturating_sub(1)))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
        }
    }
}

/// Errors that repeat deterministically on a broken server.
fn is_protocol_violation(e: &NetError) -> bool {
    matches!(
        e,
        NetError::BadContentRange { .. } | NetError::ExtraBytes { .. } | NetError::Protocol
    )
}

/// Retry policy of one worker (all bounds from `limits.rs`).
fn retries_exhausted(failures: u32, protocol_failures: u32, attempts: u32) -> bool {
    failures > limits::MAX_SEGMENT_RETRIES
        || protocol_failures > limits::MAX_PROTOCOL_RETRIES
        || attempts >= limits::MAX_TRANSFER_ATTEMPTS
}

fn release(live: &Live, slot: u64) {
    if let Some(b) = lock(&live.board).as_mut() {
        b.release(slot);
    }
}

enum TransferError {
    Stopped,
    /// Retryable network error; `bool` = some bytes were written first.
    Retry(NetError, bool),
    Fatal(EngineError),
}

/// Streams one claimed slot. `Ok(true)` when bytes were written.
async fn transfer(ctx: &WorkerCtx, slot: u64) -> Result<bool, TransferError> {
    let live = &ctx.live;
    let (pos, end, single, size) = {
        let guard = lock(&live.board);
        let Some(b) = guard.as_ref() else {
            return Err(TransferError::Stopped);
        };
        let Some(s) = b.slots().iter().find(|s| s.id == slot) else {
            return Ok(false);
        };
        (s.pos, s.range.end(), b.is_single(), b.size())
    };
    // Non-resumable downloads always restart from offset 0 without Range.
    let range = if ctx.resumable && (!single || pos > 0) {
        ByteRange::new(pos, end)
    } else {
        None
    };
    if range.is_none()
        && pos > 0
        && let Some(b) = lock(&live.board).as_mut()
    {
        b.go_single(slot);
    }

    let mut stop_rx = live.stop.subscribe();
    if live.stop_kind().is_some() {
        return Err(TransferError::Stopped);
    }
    let fetched = tokio::select! {
        r = ctx.client.fetch(&ctx.request, range, size, ctx.validator.as_ref()) => r,
        _ = stop_rx.changed() => return Err(TransferError::Stopped),
    };
    let mut body = match fetched {
        Ok(FetchBody::Partial(b)) => b,
        Ok(FetchBody::Full(b)) => {
            if range.is_some() {
                // Range ignored (200): never append; restart from 0 on one connection.
                tracing::info!("server ignored Range; falling back to one connection");
                if let Some(board) = lock(&live.board).as_mut() {
                    board.go_single(slot);
                }
            }
            b
        }
        Err(e) if e.is_retryable() => return Err(TransferError::Retry(e, false)),
        Err(e) => return Err(TransferError::Fatal(e.into())),
    };

    let mut wrote = false;
    loop {
        let chunk = tokio::select! {
            c = body.next_chunk() => c,
            _ = stop_rx.changed() => return Err(TransferError::Stopped),
        };
        let chunk = match chunk {
            Ok(Some(c)) => c,
            Ok(None) => {
                // End of stream.
                let mut guard = lock(&live.board);
                let Some(b) = guard.as_mut() else {
                    return Err(TransferError::Stopped);
                };
                if b.size().is_none() {
                    b.finish_unknown(slot);
                    return Ok(wrote);
                }
                return match b.position(slot) {
                    None => Ok(wrote),
                    Some(_) => Err(TransferError::Retry(
                        NetError::ShortBody {
                            expected: 0,
                            received: 0,
                        },
                        wrote,
                    )),
                };
            }
            Err(e) if e.is_retryable() => return Err(TransferError::Retry(e, wrote)),
            Err(e) => return Err(TransferError::Fatal(e.into())),
        };
        let mut data: &[u8] = &chunk;
        while !data.is_empty() {
            let want = u64::try_from(data.len()).unwrap_or(u64::MAX);
            let reserved = lock(&live.board)
                .as_mut()
                .and_then(|b| b.reserve(slot, want));
            let Some((offset, n)) = reserved else {
                // Slot taken away (go_single elsewhere): stop quietly.
                return Ok(wrote);
            };
            if n == 0 {
                let mut guard = lock(&live.board);
                if let Some(b) = guard.as_mut() {
                    b.commit(slot, 0);
                }
                return Ok(wrote);
            }
            ctx.global.acquire(n).await;
            live.limiter.acquire(n).await;
            let len = usize::try_from(n).unwrap_or(usize::MAX).min(data.len());
            let (head, tail) = data.split_at(len);
            if let Err(e) = ctx.writer.write_at(offset, head) {
                if let Some(b) = lock(&live.board).as_mut() {
                    b.unreserve(slot);
                }
                return Err(TransferError::Fatal(write_error(&ctx.part_path, &e)));
            }
            wrote = true;
            let closed = lock(&live.board).as_mut().is_none_or(|b| b.commit(slot, n));
            if closed {
                // Range done (possibly shrunk by a split): drop the connection.
                return Ok(wrote);
            }
            data = tail;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(etag: Option<&str>, lm: Option<&str>, size: Option<u64>) -> ResumeState {
        ResumeState {
            id: "d-000001".into(),
            final_url: "http://x/".into(),
            size,
            etag: etag.map(str::to_owned),
            last_modified: lm.map(str::to_owned),
            part_path: PathBuf::from("x.udmpart"),
            file_name: "x".into(),
            coverage: vec![],
            resumable: true,
        }
    }

    fn probe(etag: Option<&str>, lm: Option<&str>, size: Option<u64>) -> ProbeInfo {
        ProbeInfo {
            final_url: url::Url::parse("http://x/").unwrap(),
            size,
            accepts_ranges: true,
            etag: etag.and_then(EntityTag::parse),
            last_modified: lm.map(str::to_owned),
            filename: None,
            content_type: None,
            protocol: udm_net::Protocol::Http1,
        }
    }

    #[test]
    fn validators() {
        let lm = Some("Wed, 21 Oct 2015 07:28:00 GMT");
        assert!(validators_match(
            &saved(Some("\"a\""), None, Some(5)),
            &probe(Some("\"a\""), None, Some(5))
        ));
        assert!(!validators_match(
            &saved(Some("\"a\""), None, Some(5)),
            &probe(Some("\"b\""), None, Some(5))
        ));
        assert!(!validators_match(
            &saved(Some("\"a\""), lm, Some(5)),
            &probe(Some("\"a\""), lm, Some(6))
        ));
        // Weak ETag falls back to Last-Modified + size.
        assert!(validators_match(
            &saved(Some("W/\"a\""), lm, Some(5)),
            &probe(Some("W/\"a\""), lm, Some(5))
        ));
        assert!(!validators_match(
            &saved(Some("W/\"a\""), None, Some(5)),
            &probe(Some("W/\"a\""), None, Some(5))
        ));
        // Strong tag disappeared → restart.
        assert!(!validators_match(
            &saved(Some("\"a\""), lm, Some(5)),
            &probe(None, lm, Some(5))
        ));
        assert!(!validators_match(
            &saved(None, None, Some(5)),
            &probe(None, None, Some(5))
        ));
    }

    #[test]
    fn hostile_retry_limits_enforced() {
        assert!(!retries_exhausted(limits::MAX_SEGMENT_RETRIES, 0, 1));
        assert!(retries_exhausted(limits::MAX_SEGMENT_RETRIES + 1, 0, 1));
        assert!(!retries_exhausted(1, limits::MAX_PROTOCOL_RETRIES, 1));
        assert!(retries_exhausted(1, limits::MAX_PROTOCOL_RETRIES + 1, 1));
        // A server that always sends a little and then breaks resets
        // `failures` each time, but the attempt cap still ends it.
        assert!(retries_exhausted(1, 0, limits::MAX_TRANSFER_ATTEMPTS));
        assert!(is_protocol_violation(&NetError::Protocol));
        assert!(!is_protocol_violation(&NetError::Io));
    }

    #[test]
    fn backoff_is_bounded() {
        assert_eq!(
            backoff(0),
            Duration::from_millis(limits::RETRY_BACKOFF_BASE_MS)
        );
        assert_eq!(
            backoff(1_000),
            Duration::from_millis(limits::RETRY_BACKOFF_MAX_MS)
        );
    }
}

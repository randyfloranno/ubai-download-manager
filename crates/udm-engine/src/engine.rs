//! The engine: owns downloads, queues, settings and history, runs the
//! background loop (queue pump, scheduler, clipboard watcher, autosave) and
//! supervises download tasks.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::broadcast;
use udm_core::{DownloadId, DownloadStatus, Message, PauseReason, Secret, limits};
use udm_net::{Client, NetConfig, ProxyConfig, Request, TlsException};
use udm_proto::ChecksumSpec;
use udm_secrets::{SecretRef, SecretStore};
use udm_store::{DataDir, HistoryLog, load_envelope, save_envelope};

use crate::error::{EngineError, EngineResult};
use crate::grabber::GrabJob;
use crate::limiter::Limiter;
use crate::model::{
    DownloadRecord, DownloadView, HistoryEntry, QueueFile, QueueRecord, SegmentView, StoredMessage,
    StoredStatus,
};
use crate::platform::Platform;
use crate::schedule::{AfterDone, Schedule, ScheduleEvent};
use crate::settings::{PROXY_SECRET_REF, ProxyKind, Settings};
use crate::task::{self, Live, Outcome, Phase, StopKind, TaskInput, lock};

/// Envelope kind and schema of the queue file.
pub const QUEUE_KIND: &str = "queue";
/// Schema of the queue file.
pub const QUEUE_SCHEMA: u32 = 1;
/// Envelope kind of the settings file.
pub const SETTINGS_KIND: &str = "settings";
/// Schema of the settings file.
pub const SETTINGS_SCHEMA: u32 = 1;
/// Id of the default queue.
pub const MAIN_QUEUE: &str = "main";

const TICK_MS: u64 = 250;
const SHUTDOWN_WAIT_MS: u64 = 10_000;

/// Something the front-ends may want to show.
#[derive(Clone, Debug)]
pub enum EngineEvent {
    /// A download finished.
    Completed {
        /// Id.
        id: DownloadId,
        /// File name.
        file_name: String,
        /// Final path.
        path: PathBuf,
    },
    /// A download failed.
    Failed {
        /// Id.
        id: DownloadId,
        /// Reason.
        message: Message,
    },
    /// Informational notice.
    Notice {
        /// Download concerned, if any.
        id: Option<DownloadId>,
        /// Text.
        message: Message,
    },
    /// The scheduler asked the app to exit.
    ExitRequested,
    /// Links found on the clipboard (clipboard capture enabled).
    ClipboardLinks(Vec<String>),
}

/// How the engine is constructed.
#[derive(Clone)]
pub struct EngineConfig {
    /// Data dir.
    pub data_dir: DataDir,
    /// OS seams.
    pub platform: Arc<dyn Platform>,
    /// Keychain.
    pub secrets: Arc<dyn SecretStore>,
    /// Extra trusted roots (tests, `--sample`).
    pub extra_root_der: Vec<Vec<u8>>,
    /// Network timeouts override (tests).
    pub timeouts: Option<udm_net::Timeouts>,
}

pub(crate) struct Entry {
    pub(crate) rec: DownloadRecord,
    pub(crate) status: DownloadStatus,
    pub(crate) live: Option<Arc<Live>>,
    /// Stop requested with removal: delete the record when the task ends.
    pub(crate) removing: Option<bool>,
}

pub(crate) struct State {
    pub(crate) settings: Settings,
    pub(crate) entries: Vec<Entry>,
    pub(crate) queues: Vec<QueueRecord>,
    pub(crate) next_id: u64,
    pub(crate) history: Vec<HistoryEntry>,
    pub(crate) grab_jobs: Vec<GrabJob>,
    pub(crate) next_job: u64,
    /// Completions per queue since it was last started.
    pub(crate) queue_done: HashMap<String, u32>,
    pub(crate) clipboard_last: Option<String>,
    pub(crate) dirty_queue: Option<Instant>,
    pub(crate) dirty_settings: Option<Instant>,
    pub(crate) last_schedule_check: chrono::NaiveDateTime,
}

pub(crate) struct Inner {
    pub(crate) state: Mutex<State>,
    pub(crate) cfg: EngineConfig,
    pub(crate) global: Arc<Limiter>,
    pub(crate) client: Mutex<Arc<Client>>,
    pub(crate) events: broadcast::Sender<EngineEvent>,
    pub(crate) history_log: Mutex<Option<HistoryLog<HistoryEntry>>>,
    pub(crate) shutting_down: AtomicBool,
}

/// Cheap, cloneable handle to the engine.
#[derive(Clone)]
pub struct Engine {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Engine")
    }
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

fn local_now() -> chrono::NaiveDateTime {
    chrono::Local::now().naive_local()
}

/// Moves a corrupt file aside so autosave does not destroy the evidence.
fn quarantine(path: &std::path::Path) {
    let mut target = path.as_os_str().to_owned();
    target.push(format!(".corrupt-{}", now_unix()));
    if let Err(e) = std::fs::rename(path, PathBuf::from(target)) {
        tracing::warn!("cannot move corrupt file aside: {e}");
    }
}

fn default_queue() -> QueueRecord {
    QueueRecord {
        id: MAIN_QUEUE.to_owned(),
        name: String::new(),
        concurrency: 3,
        running: false,
        schedule: Schedule::default(),
        items: Vec::new(),
    }
}

impl Engine {
    /// Loads state from the data dir and starts the background loop.
    /// Must be called inside a tokio runtime. Returns notices about files
    /// that were damaged and not used.
    pub fn start(cfg: EngineConfig) -> EngineResult<(Self, Vec<Message>)> {
        cfg.data_dir.ensure()?;
        let mut notices = Vec::new();

        let settings_path = cfg.data_dir.settings_file();
        let mut settings =
            match load_envelope::<Settings>(&settings_path, SETTINGS_KIND, SETTINGS_SCHEMA) {
                Ok(Some((_, s))) => match s.validate() {
                    Ok(()) => s,
                    Err(m) => {
                        notices.push(m);
                        quarantine(&settings_path);
                        Settings::default()
                    }
                },
                Ok(None) => Settings::default(),
                Err(e) => {
                    notices.push(e.to_message());
                    quarantine(&settings_path);
                    Settings::default()
                }
            };

        if let Some(home) = udm_store::user_home() {
            settings.resolve_relative_dirs(&home);
        }

        let queue_path = cfg.data_dir.queue_file();
        let qf = match load_envelope::<QueueFile>(&queue_path, QUEUE_KIND, QUEUE_SCHEMA) {
            Ok(Some((_, q))) => q,
            Ok(None) => QueueFile::default(),
            Err(e) => {
                notices.push(e.to_message());
                quarantine(&queue_path);
                QueueFile::default()
            }
        };

        let (log, history, skipped) =
            match HistoryLog::<HistoryEntry>::open(&cfg.data_dir.history_file()) {
                Ok((l, h, s)) => (Some(l), h, s),
                Err(e) => {
                    notices.push(e.to_message());
                    (None, Vec::new(), 0)
                }
            };
        if skipped > 0 {
            notices.push(
                Message::new("notice-history-lines-skipped")
                    .with_uint("count", u64::try_from(skipped).unwrap_or(u64::MAX)),
            );
        }

        let mut queues: Vec<QueueRecord> = qf.queues.into_iter().take(limits::MAX_QUEUES).collect();
        if !queues.iter().any(|q| q.id == MAIN_QUEUE) {
            queues.insert(0, default_queue());
        }
        let mut entries = Vec::new();
        let mut max_id = 0u64;
        for mut rec in qf.downloads.into_iter().take(limits::MAX_QUEUE_SIZE) {
            let Some(id) = rec.download_id() else {
                continue;
            };
            max_id = max_id.max(id.value());
            let mut status = rec.status.to_status();
            // Downloads interrupted by shutdown resume on the next start.
            if status == DownloadStatus::Paused(PauseReason::Shutdown) {
                status = DownloadStatus::Queued;
                rec.manual_start = true;
            }
            if !queues.iter().any(|q| q.id == rec.queue) {
                rec.queue = MAIN_QUEUE.to_owned();
            }
            entries.push(Entry {
                rec,
                status,
                live: None,
                removing: None,
            });
        }
        // Queue items must reference existing downloads, once each.
        for q in &mut queues {
            let mut seen = std::collections::HashSet::new();
            q.items.retain(|i| {
                entries
                    .iter()
                    .any(|e| &e.rec.id == i && e.rec.queue == q.id)
                    && seen.insert(i.clone())
            });
        }
        for e in &entries {
            if let Some(q) = queues.iter_mut().find(|q| q.id == e.rec.queue)
                && !q.items.contains(&e.rec.id)
            {
                q.items.push(e.rec.id.clone());
            }
        }

        let global = Arc::new(Limiter::new(settings.global_speed_limit));
        let client = Arc::new(build_client(&cfg, &settings)?);
        let (events, _) = broadcast::channel(256);
        let state = State {
            settings,
            entries,
            queues,
            next_id: qf.next_id.max(max_id.saturating_add(1)).max(1),
            history,
            grab_jobs: Vec::new(),
            next_job: 1,
            queue_done: HashMap::new(),
            clipboard_last: None,
            dirty_queue: None,
            dirty_settings: None,
            last_schedule_check: local_now(),
        };
        let engine = Engine {
            inner: Arc::new(Inner {
                state: Mutex::new(state),
                cfg,
                global,
                client: Mutex::new(client),
                events,
                history_log: Mutex::new(log),
                shutting_down: AtomicBool::new(false),
            }),
        };
        let bg = engine.clone();
        tokio::spawn(async move { bg.background().await });
        engine.pump();
        Ok((engine, notices))
    }

    /// Subscribes to events.
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    pub(crate) fn emit(&self, ev: EngineEvent) {
        // No receiver is fine.
        let _ = self.inner.events.send(ev);
    }

    pub(crate) fn state(&self) -> std::sync::MutexGuard<'_, State> {
        lock(&self.inner.state)
    }

    pub(crate) fn mark_dirty(st: &mut State) {
        if st.dirty_queue.is_none() {
            st.dirty_queue = Some(Instant::now());
        }
    }

    pub(crate) fn mark_settings_dirty(st: &mut State) {
        if st.dirty_settings.is_none() {
            st.dirty_settings = Some(Instant::now());
        }
    }

    pub(crate) fn client(&self) -> Arc<Client> {
        Arc::clone(&lock(&self.inner.client))
    }

    /// Rebuilds the network client after proxy/TLS/user-agent changes.
    pub(crate) fn rebuild_client(&self) -> EngineResult<()> {
        let settings = self.state().settings.clone();
        let c = build_client(&self.inner.cfg, &settings)?;
        *lock(&self.inner.client) = Arc::new(c);
        Ok(())
    }

    /// The data dir.
    pub fn data_dir(&self) -> &DataDir {
        &self.inner.cfg.data_dir
    }

    // ---------------------------------------------------------------
    // Background loop
    // ---------------------------------------------------------------

    async fn background(&self) {
        let mut tick = tokio::time::interval(Duration::from_millis(TICK_MS));
        let mut n: u64 = 0;
        loop {
            tick.tick().await;
            if self.inner.shutting_down.load(Ordering::SeqCst) {
                break;
            }
            n = n.wrapping_add(1);
            let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.sync_live_facts();
                self.autosave(false);
                if n.is_multiple_of(4) {
                    self.scheduler_tick(local_now());
                    self.clipboard_tick();
                    self.pump();
                }
            }));
            if guarded.is_err() {
                tracing::error!("engine background tick panicked; continuing");
            }
        }
    }

    /// Copies the name, folder and size a running task has chosen into its
    /// record, so a crash (no task-end handler) still restarts on the same
    /// part file and removal can find it.
    pub(crate) fn sync_live_facts(&self) {
        let mut st = self.state();
        let mut changed = false;
        for e in &mut st.entries {
            let Some(l) = &e.live else { continue };
            let f = lock(&l.facts).clone();
            if f.file_name.is_some() && f.file_name != e.rec.file_name {
                e.rec.file_name = f.file_name;
                changed = true;
            }
            if let Some(d) = f.dir
                && d != e.rec.dir
            {
                e.rec.dir = d;
                changed = true;
            }
            if f.size.is_some() && f.size != e.rec.size {
                e.rec.size = f.size;
                changed = true;
            }
            if let Some(c) = f.category
                && e.rec.category.is_empty()
            {
                e.rec.category = c;
                changed = true;
            }
        }
        if changed {
            Self::mark_dirty(&mut st);
        }
    }

    /// Saves dirty state when the debounce elapsed (or now when `force`).
    pub(crate) fn autosave(&self, force: bool) {
        let debounce = Duration::from_millis(limits::AUTOSAVE_DEBOUNCE_MS);
        let (queue, settings) = {
            let mut st = self.state();
            let due = |d: Option<Instant>| d.is_some_and(|t| force || t.elapsed() >= debounce);
            let q = if due(st.dirty_queue) {
                st.dirty_queue = None;
                Some(queue_file(&st))
            } else {
                None
            };
            let s = if due(st.dirty_settings) {
                st.dirty_settings = None;
                Some(st.settings.clone())
            } else {
                None
            };
            (q, s)
        };
        let dd = &self.inner.cfg.data_dir;
        if let Some(q) = queue
            && let Err(e) = save_envelope(&dd.queue_file(), QUEUE_KIND, QUEUE_SCHEMA, &q)
        {
            tracing::error!("saving queue failed: {e}");
            Self::mark_dirty(&mut self.state());
        }
        if let Some(s) = settings
            && let Err(e) = save_envelope(&dd.settings_file(), SETTINGS_KIND, SETTINGS_SCHEMA, &s)
        {
            tracing::error!("saving settings failed: {e}");
            Self::mark_settings_dirty(&mut self.state());
        }
    }

    /// Runs scheduler boundaries between the last check and `now`.
    pub fn scheduler_tick(&self, now: chrono::NaiveDateTime) {
        let mut stops = Vec::new();
        {
            let mut st = self.state();
            let prev = st.last_schedule_check;
            st.last_schedule_check = now;
            let mut changed = false;
            let mut started = Vec::new();
            for q in &mut st.queues {
                for ev in q.schedule.due_events(prev, now) {
                    match ev {
                        ScheduleEvent::Start => {
                            if !q.running {
                                q.running = true;
                                changed = true;
                                started.push(q.id.clone());
                            }
                        }
                        ScheduleEvent::Stop => {
                            if q.running {
                                q.running = false;
                                changed = true;
                            }
                            stops.push(q.id.clone());
                        }
                    }
                }
            }
            for id in started {
                st.queue_done.insert(id, 0);
            }
            // Requeue running downloads of stopped queues (not manual ones).
            for qid in &stops {
                for e in st
                    .entries
                    .iter_mut()
                    .filter(|e| &e.rec.queue == qid && !e.rec.manual_start)
                {
                    if let Some(l) = &e.live {
                        l.request_stop(StopKind::Requeue);
                    }
                }
            }
            if changed {
                Self::mark_dirty(&mut st);
            }
        }
        if !stops.is_empty() {
            self.pump();
        }
    }

    fn clipboard_tick(&self) {
        let enabled = self.state().settings.clipboard_capture;
        if !enabled {
            return;
        }
        let text = match self.inner.cfg.platform.clipboard_text() {
            Ok(Some(t)) => t,
            Ok(None) => return,
            Err(e) => {
                tracing::debug!("clipboard: {e}");
                return;
            }
        };
        let links = {
            let mut st = self.state();
            if st.clipboard_last.as_deref() == Some(text.as_str()) {
                return;
            }
            st.clipboard_last = Some(text.clone());
            let exts = st.settings.capture_extensions.clone();
            udm_proto::extract_urls_from_text(&text)
                .into_iter()
                .filter(|u| link_matches_extensions(u, &exts))
                .collect::<Vec<_>>()
        };
        if !links.is_empty() {
            self.emit(EngineEvent::ClipboardLinks(links));
        }
    }

    /// Starts whatever may start now and finishes queues that ran dry.
    pub(crate) fn pump(&self) {
        if self.inner.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let mut to_start: Vec<DownloadId> = Vec::new();
        let mut after: Vec<(String, AfterDone)> = Vec::new();
        {
            let mut st = self.state();
            let max_total = st
                .settings
                .max_concurrent
                .clamp(1, limits::MAX_CONCURRENT_DOWNLOADS);
            let mut running = u32::try_from(st.entries.iter().filter(|e| e.live.is_some()).count())
                .unwrap_or(u32::MAX);
            // Manual starts first, in list order.
            for e in &st.entries {
                if running >= max_total {
                    break;
                }
                if e.live.is_none()
                    && e.removing.is_none()
                    && e.status == DownloadStatus::Queued
                    && e.rec.manual_start
                    && let Some(id) = e.rec.download_id()
                {
                    to_start.push(id);
                    running = running.saturating_add(1);
                }
            }
            // Then running queues, in queue order.
            let queues = st.queues.clone();
            for q in queues.iter().filter(|q| q.running) {
                let mut active = u32::try_from(
                    st.entries
                        .iter()
                        .filter(|e| e.rec.queue == q.id && e.live.is_some())
                        .count(),
                )
                .unwrap_or(u32::MAX)
                .saturating_add(
                    u32::try_from(
                        to_start
                            .iter()
                            .filter(|id| {
                                st.entries.iter().any(|e| {
                                    e.rec.queue == q.id && e.rec.download_id() == Some(**id)
                                })
                            })
                            .count(),
                    )
                    .unwrap_or(0),
                );
                let mut pending = false;
                for item in &q.items {
                    let Some(e) = st.entries.iter().find(|e| &e.rec.id == item) else {
                        continue;
                    };
                    if e.removing.is_some() {
                        continue;
                    }
                    let waiting = e.live.is_none() && e.status == DownloadStatus::Queued;
                    if e.live.is_some() || waiting {
                        pending = true;
                    }
                    if waiting
                        && active < q.concurrency.max(1)
                        && running < limits::MAX_CONCURRENT_DOWNLOADS
                        && let Some(id) = e.rec.download_id()
                        && !to_start.contains(&id)
                    {
                        to_start.push(id);
                        active = active.saturating_add(1);
                        running = running.saturating_add(1);
                    }
                }
                if !pending {
                    // Queue finished.
                    let done = st.queue_done.get(&q.id).copied().unwrap_or(0);
                    if let Some(qm) = st.queues.iter_mut().find(|x| x.id == q.id) {
                        qm.running = false;
                    }
                    Self::mark_dirty(&mut st);
                    if done > 0 {
                        after.push((q.id.clone(), q.schedule.after_done));
                    }
                    st.queue_done.insert(q.id.clone(), 0);
                }
            }
        }
        for id in to_start {
            self.spawn_download(id);
        }
        for (qid, action) in after {
            self.run_after_done(&qid, action);
        }
    }

    fn run_after_done(&self, queue: &str, action: AfterDone) {
        self.emit(EngineEvent::Notice {
            id: None,
            message: Message::new("notice-queue-finished").with_str("queue", queue),
        });
        let power = match action {
            AfterDone::None => return,
            AfterDone::Exit => {
                self.emit(EngineEvent::ExitRequested);
                return;
            }
            AfterDone::Shutdown => udm_sys::PowerAction::Shutdown,
            AfterDone::Sleep => udm_sys::PowerAction::Sleep,
            AfterDone::Hibernate => udm_sys::PowerAction::Hibernate,
        };
        // Save everything before the machine goes down.
        self.autosave(true);
        if let Err(e) = self.inner.cfg.platform.power(power) {
            self.emit(EngineEvent::Notice {
                id: None,
                message: e.to_message(),
            });
        }
    }

    // ---------------------------------------------------------------
    // Task supervision
    // ---------------------------------------------------------------

    fn secret(&self, r: &str) -> Option<Secret> {
        let r = SecretRef::new(r).ok()?;
        match self.inner.cfg.secrets.get(&r) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("keychain read failed: {}", e.to_message());
                None
            }
        }
    }

    pub(crate) fn put_secret(&self, r: &str, s: &Secret) -> EngineResult<()> {
        let r = SecretRef::new(r).map_err(|e| EngineError::Msg(e.to_message()))?;
        self.inner
            .cfg
            .secrets
            .put(&r, s)
            .map_err(|e| EngineError::Msg(e.to_message()))
    }

    pub(crate) fn delete_secret(&self, r: &str) {
        if let Ok(r) = SecretRef::new(r)
            && let Err(e) = self.inner.cfg.secrets.delete(&r)
        {
            tracing::warn!("keychain delete failed: {}", e.to_message());
        }
    }

    /// Starts the task of one queued download.
    pub(crate) fn spawn_download(&self, id: DownloadId) {
        let prepared = {
            let mut st = self.state();
            let settings = st.settings.clone();
            let Some(e) = st
                .entries
                .iter_mut()
                .find(|e| e.rec.download_id() == Some(id))
            else {
                return;
            };
            if e.live.is_some() {
                return;
            }
            let url = match udm_proto::validate_download_url(&e.rec.url) {
                Ok(u) => u,
                Err(err) => {
                    e.status = DownloadStatus::Failed(err.to_message());
                    Self::mark_dirty(&mut st);
                    return;
                }
            };
            let live = Arc::new(Live::new(
                e.rec
                    .connections
                    .clamp(1, limits::MAX_CONNECTIONS_PER_DOWNLOAD),
                e.rec.speed_limit,
            ));
            e.live = Some(Arc::clone(&live));
            e.status = DownloadStatus::Probing;
            e.rec.notice = None;
            let rec = e.rec.clone();
            Self::mark_dirty(&mut st);
            (rec, url, live, settings)
        };
        let (rec, url, live, settings) = prepared;

        let mut request = Request::new(url.clone());
        request.referer = rec.referer.clone();
        request.allow_downgrade = rec.allow_downgrade;
        if let Some(r) = &rec.cookie_ref {
            request.cookie = self.secret(r);
        }
        if let Some(user) = &rec.auth_user {
            if let Some(p) = self.secret(&DownloadRecord::auth_ref(&rec.id)) {
                request.credentials = Some((user.clone(), p));
            }
        } else if let Some(sa) = url.host_str().and_then(|h| settings.site_auth_for(h))
            && let Some(p) = self.secret(&sa.secret_ref())
        {
            request.credentials = Some((sa.user.clone(), p));
        }
        let checksum = rec
            .checksum
            .as_deref()
            .and_then(|c| ChecksumSpec::parse(c).ok());
        let folders_settings = settings.clone();
        let folders: task::FolderFn = Arc::new(move |cat: Option<&str>, name: &str| {
            let c = cat.map_or_else(|| folders_settings.classify(name), str::to_owned);
            let d = folders_settings.folder_for(&c);
            (c, d)
        });
        let dir = if rec.dir.as_os_str().is_empty() {
            None
        } else {
            Some(rec.dir.clone())
        };
        let category = if rec.category.is_empty() {
            None
        } else {
            Some(rec.category.clone())
        };
        let input = TaskInput {
            id,
            request,
            dir,
            file_name: rec.file_name.clone().or_else(|| rec.requested_name.clone()),
            name_from_user: rec.requested_name.is_some(),
            category,
            checksum,
            client: self.client(),
            global: Arc::clone(&self.inner.global),
            platform: Arc::clone(&self.inner.cfg.platform),
            data_dir: self.inner.cfg.data_dir.clone(),
            folders,
        };
        let engine = self.clone();
        let live2 = Arc::clone(&live);
        let handle = tokio::spawn(task::run(input, live2));
        tokio::spawn(async move {
            let outcome = match handle.await {
                Ok(o) => o,
                Err(join) => {
                    tracing::error!(%id, "download task panicked: {join}");
                    Outcome::Failed(Message::new("error-engine-internal"))
                }
            };
            engine.on_task_end(id, &live, outcome);
        });
    }

    fn on_task_end(&self, id: DownloadId, live: &Arc<Live>, outcome: Outcome) {
        let facts = lock(&live.facts).clone();
        let mut events = Vec::new();
        let mut secrets_del: Vec<String> = Vec::new();
        let mut delete: Option<(PathBuf, bool)> = None;
        {
            let mut st = self.state();
            let shutting = self.inner.shutting_down.load(Ordering::SeqCst);
            let Some(idx) = st
                .entries
                .iter()
                .position(|e| e.rec.download_id() == Some(id))
            else {
                return;
            };
            let mut history_add = None;
            let mut done_queue = None;
            if let Some(e) = st.entries.get_mut(idx) {
                e.live = None;
                if let Some(s) = facts.size {
                    e.rec.size = Some(s);
                }
                if let Some(n) = &facts.file_name {
                    e.rec.file_name = Some(n.clone());
                }
                if let Some(d) = &facts.dir {
                    e.rec.dir = d.clone();
                }
                if let Some(c) = &facts.category
                    && e.rec.category.is_empty()
                {
                    e.rec.category = c.clone();
                }
                if let Some(r) = facts.resumable {
                    e.rec.resumable = r;
                }
                if let Some(n) = &facts.notice {
                    e.rec.notice = Some(StoredMessage::from(n));
                    events.push(EngineEvent::Notice {
                        id: Some(id),
                        message: n.clone(),
                    });
                }
                e.rec.downloaded = lock(&live.board)
                    .as_ref()
                    .map_or(e.rec.downloaded, crate::board::Board::downloaded);
                match outcome {
                    Outcome::Completed { path, size, sha256 } => {
                        e.status = DownloadStatus::Completed;
                        e.rec.manual_start = false;
                        e.rec.downloaded = size;
                        e.rec.size = Some(size);
                        let ts = now_unix();
                        e.rec.completed_at = Some(ts);
                        let name = e.rec.file_name.clone().unwrap_or_default();
                        history_add = Some(HistoryEntry {
                            id: e.rec.id.clone(),
                            url: e.rec.url.clone(),
                            path: path.display().to_string(),
                            file_name: name.clone(),
                            size,
                            category: e.rec.category.clone(),
                            completed_at: ts,
                            sha256,
                        });
                        done_queue = Some(e.rec.queue.clone());
                        if let Some(r) = e.rec.cookie_ref.take() {
                            secrets_del.push(r);
                        }
                        events.push(EngineEvent::Completed {
                            id,
                            file_name: name,
                            path,
                        });
                    }
                    Outcome::Stopped(StopKind::Pause(r)) => {
                        e.status = DownloadStatus::Paused(r);
                        if r == PauseReason::User {
                            e.rec.manual_start = false;
                        }
                    }
                    Outcome::Stopped(StopKind::Requeue) => {
                        e.status = DownloadStatus::Queued;
                    }
                    Outcome::Stopped(StopKind::Remove { delete_file }) => {
                        let part = e
                            .rec
                            .file_name
                            .as_ref()
                            .map(|n| e.rec.dir.join(format!("{n}{}", task::PART_SUFFIX)));
                        if let Some(p) = part {
                            delete = Some((p, delete_file));
                        }
                        e.removing = Some(delete_file);
                    }
                    Outcome::Paused(r, m) => {
                        e.status = DownloadStatus::Paused(r);
                        e.rec.notice = Some(StoredMessage::from(&m));
                        events.push(EngineEvent::Notice {
                            id: Some(id),
                            message: m,
                        });
                    }
                    Outcome::Failed(m) => {
                        if shutting {
                            e.status = DownloadStatus::Paused(PauseReason::Shutdown);
                        } else {
                            e.status = DownloadStatus::Failed(m.clone());
                            e.rec.manual_start = false;
                            events.push(EngineEvent::Failed { id, message: m });
                        }
                    }
                }
                // A remove can race with the task ending some other way (failed while
                // probing, paused by an error): the part file still has to go.
                if let Some(delete_file) = e.removing
                    && delete.is_none()
                    && let Some(n) = &e.rec.file_name
                {
                    delete = Some((
                        e.rec.dir.join(format!("{n}{}", task::PART_SUFFIX)),
                        delete_file,
                    ));
                }
                e.rec.status = StoredStatus::from_status(&e.status);
            }
            if st.entries.get(idx).is_some_and(|e| e.removing.is_some()) {
                let removed = st.entries.remove(idx);
                for q in &mut st.queues {
                    q.items.retain(|i| i != &removed.rec.id);
                }
                secrets_del.extend(removed.rec.cookie_ref);
                if removed.rec.auth_user.is_some() {
                    secrets_del.push(DownloadRecord::auth_ref(&removed.rec.id));
                }
            }
            if let Some(q) = done_queue {
                let c = st.queue_done.entry(q).or_insert(0);
                *c = c.saturating_add(1);
            }
            if let Some(h) = history_add {
                st.history.push(h.clone());
                if st.history.len() > limits::MAX_HISTORY_RECORDS {
                    st.history.remove(0);
                }
                drop(st);
                if let Some(log) = lock(&self.inner.history_log).as_mut()
                    && let Err(e) = log.append(&h)
                {
                    tracing::error!("history append failed: {e}");
                }
                st = self.state();
            }
            Self::mark_dirty(&mut st);
        }
        if let Some((part, delete_file)) = delete {
            let _ = std::fs::remove_file(self.inner.cfg.data_dir.state_file(id));
            if delete_file {
                crate::commands::remove_part(&part);
            }
        }
        for r in &secrets_del {
            self.delete_secret(r);
        }
        for ev in events {
            self.emit(ev);
        }
        self.pump();
    }

    /// Stops every task (state preserved), waits for them and saves.
    pub async fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        let lives: Vec<Arc<Live>> = self
            .state()
            .entries
            .iter()
            .filter_map(|e| e.live.clone())
            .collect();
        for l in &lives {
            l.request_stop(StopKind::Pause(PauseReason::Shutdown));
        }
        let now = Instant::now();
        let deadline = now
            .checked_add(Duration::from_millis(SHUTDOWN_WAIT_MS))
            .unwrap_or(now);
        while Instant::now() < deadline && self.state().entries.iter().any(|e| e.live.is_some()) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        {
            let mut st = self.state();
            Self::mark_dirty(&mut st);
            Self::mark_settings_dirty(&mut st);
        }
        self.autosave(true);
    }

    // ---------------------------------------------------------------
    // Views
    // ---------------------------------------------------------------

    /// All downloads, in list order.
    pub fn downloads(&self) -> Vec<DownloadView> {
        let st = self.state();
        st.entries.iter().map(|e| view(&st, e)).collect()
    }

    /// One download.
    pub fn download(&self, id: DownloadId) -> Option<DownloadView> {
        let st = self.state();
        st.entries
            .iter()
            .find(|e| e.rec.download_id() == Some(id))
            .map(|e| view(&st, e))
    }

    /// Current settings.
    pub fn settings(&self) -> Settings {
        self.state().settings.clone()
    }

    /// Queues.
    pub fn queues(&self) -> Vec<QueueRecord> {
        self.state().queues.clone()
    }

    /// History, newest first, optionally filtered (case-insensitive).
    pub fn history(&self, search: Option<&str>) -> Vec<HistoryEntry> {
        let st = self.state();
        let needle = search.map(str::to_lowercase);
        st.history
            .iter()
            .rev()
            .filter(|h| {
                needle.as_ref().is_none_or(|n| {
                    h.file_name.to_lowercase().contains(n) || h.url.to_lowercase().contains(n)
                })
            })
            .cloned()
            .collect()
    }

    /// Totals for the status bar.
    pub fn stats(&self) -> Stats {
        let st = self.state();
        let mut s = Stats {
            total: u64::try_from(st.entries.len()).unwrap_or(u64::MAX),
            global_speed_limit: st.settings.global_speed_limit,
            ..Stats::default()
        };
        for e in &st.entries {
            if let Some(l) = &e.live {
                s.active = s.active.saturating_add(1);
                s.speed = s.speed.saturating_add(l.speed.load(Ordering::Relaxed));
            }
            match e.status {
                DownloadStatus::Completed => s.completed = s.completed.saturating_add(1),
                DownloadStatus::Failed(_) => s.failed = s.failed.saturating_add(1),
                _ => {}
            }
        }
        s
    }
}

/// Status bar totals.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Stats {
    /// Downloads in the list.
    pub total: u64,
    /// Running.
    pub active: u64,
    /// Completed.
    pub completed: u64,
    /// Failed.
    pub failed: u64,
    /// Sum of speeds.
    pub speed: u64,
    /// Global limit.
    pub global_speed_limit: u64,
}

fn queue_file(st: &State) -> QueueFile {
    QueueFile {
        next_id: st.next_id,
        downloads: st
            .entries
            .iter()
            .filter(|e| e.removing.is_none())
            .map(|e| {
                let mut r = e.rec.clone();
                r.status = StoredStatus::from_status(&e.status);
                if let Some(l) = &e.live {
                    r.downloaded = lock(&l.board)
                        .as_ref()
                        .map_or(r.downloaded, crate::board::Board::downloaded);
                    // A running download resumes after a crash.
                    r.manual_start =
                        r.manual_start || !st.queues.iter().any(|q| q.id == r.queue && q.running);
                }
                r
            })
            .collect(),
        queues: st.queues.clone(),
    }
}

pub(crate) fn link_matches_extensions(u: &str, exts: &[String]) -> bool {
    if exts.is_empty() {
        return true;
    }
    let Ok(url) = url::Url::parse(u) else {
        return false;
    };
    let name = udm_proto::filename_from_url(&url).unwrap_or_default();
    udm_proto::extension(&name).is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(&e)))
}

fn view(st: &State, e: &Entry) -> DownloadView {
    let mut status = e.status.clone();
    let mut downloaded = e.rec.downloaded;
    let mut speed = 0;
    let mut active = 0;
    let mut segments = Vec::new();
    let mut size = e.rec.size;
    let mut file_name = e.rec.file_name.clone();
    let mut dir = e.rec.dir.clone();
    let mut resumable = e.rec.resumable;
    if let Some(l) = &e.live {
        status = match l.phase() {
            Phase::Probing => DownloadStatus::Probing,
            Phase::Downloading => DownloadStatus::Downloading,
            Phase::Finalising => DownloadStatus::Finalising,
        };
        speed = l.speed.load(Ordering::Relaxed);
        active = l.active.load(Ordering::Relaxed);
        if let Some(b) = lock(&l.board).as_ref() {
            downloaded = b.downloaded();
            segments = b
                .slots()
                .iter()
                .map(|s| SegmentView {
                    start: s.range.start(),
                    end: s.range.end(),
                    pos: s.pos,
                })
                .collect();
        }
        let f = lock(&l.facts);
        size = f.size.or(size);
        file_name = f.file_name.clone().or(file_name);
        dir = f.dir.clone().unwrap_or(dir);
        resumable = f.resumable.unwrap_or(resumable);
    }
    let queue_running = st.queues.iter().any(|q| q.id == e.rec.queue && q.running);
    let queue_scheduled = st
        .queues
        .iter()
        .any(|q| q.id == e.rec.queue && q.schedule.start_at.is_some());
    if status == DownloadStatus::Queued && !queue_running && !e.rec.manual_start && queue_scheduled
    {
        status = DownloadStatus::Scheduled;
    }
    let message = match &status {
        DownloadStatus::Failed(m) => Some(StoredMessage::from(m)),
        _ => e.rec.notice.clone(),
    };
    let eta_secs = match (size, speed) {
        (Some(s), sp) => s.saturating_sub(downloaded).checked_div(sp),
        _ => None,
    };
    let name = file_name
        .or_else(|| e.rec.requested_name.clone())
        .or_else(|| {
            url::Url::parse(&e.rec.url)
                .ok()
                .and_then(|u| udm_proto::filename_from_url(&u))
        })
        .unwrap_or_default();
    DownloadView {
        id: e.rec.id.clone(),
        url: e.rec.url.clone(),
        dangerous_name: udm_proto::has_dangerous_double_extension(&name),
        file_name: name,
        dir: dir.display().to_string(),
        category: e.rec.category.clone(),
        queue: e.rec.queue.clone(),
        status: status.as_str().to_owned(),
        status_key: status.label_key().to_owned(),
        message,
        size,
        downloaded,
        speed,
        eta_secs,
        active_connections: active,
        connections: e
            .live
            .as_ref()
            .map_or(e.rec.connections, |l| l.connections.load(Ordering::Relaxed)),
        speed_limit: e
            .live
            .as_ref()
            .map_or(e.rec.speed_limit, |l| l.limiter.rate()),
        resumable,
        checksum: e.rec.checksum.clone(),
        added_at: e.rec.added_at,
        completed_at: e.rec.completed_at,
        segments,
    }
}

pub(crate) fn build_client(cfg: &EngineConfig, settings: &Settings) -> EngineResult<Client> {
    let proxy_secret = || -> Option<Secret> {
        let r = SecretRef::new(PROXY_SECRET_REF).ok()?;
        cfg.secrets.get(&r).ok().flatten()
    };
    let p = &settings.proxy;
    let auth = p
        .user
        .clone()
        .map(|u| (u, proxy_secret().unwrap_or_default()));
    let proxy = match p.kind {
        ProxyKind::None => None,
        ProxyKind::Http => Some(ProxyConfig::Http {
            host: p.host.clone(),
            port: p.port,
            auth,
        }),
        ProxyKind::Socks5 => Some(ProxyConfig::Socks5 {
            host: p.host.clone(),
            port: p.port,
            auth,
        }),
    };
    let net = NetConfig {
        user_agent: settings.user_agent.clone(),
        proxy,
        tls_exceptions: settings
            .tls_exceptions
            .iter()
            .map(|t| TlsException {
                host: t.host.clone(),
                sha256_fingerprint: t.sha256_fingerprint.clone(),
            })
            .collect(),
        extra_root_der: cfg.extra_root_der.clone(),
        timeouts: cfg.timeouts.unwrap_or_default(),
        allow_downgrade: false,
    };
    Ok(Client::new(net)?)
}

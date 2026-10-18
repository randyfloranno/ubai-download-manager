//! Command handlers. Every user action goes through [`Engine::execute`];
//! the UI, CLI, control port, native host and scheduler share it.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use udm_core::{CommandArgs, DownloadId, DownloadStatus, Message, PauseReason, Secret, limits};
use udm_proto::ChecksumSpec;

use crate::checksum::hash_file;
use crate::engine::{Engine, EngineEvent, Entry, MAIN_QUEUE, link_matches_extensions};
use crate::error::{EngineError, EngineResult};
use crate::grabber::{GrabJob, GrabOptions, crawl};
use crate::model::{DownloadRecord, QueueRecord, StoredMessage, StoredStatus};
use crate::registry;
use crate::schedule::{AfterDone, Schedule, parse_days, parse_time_of_day};
use crate::settings::{PROXY_SECRET_REF, ProxyKind, SiteAuth, TlsExceptionSetting};
use crate::task::{self, PART_SUFFIX, StopKind};

fn err(key: &str) -> EngineError {
    EngineError::key(key)
}

fn bad_arg(name: &str) -> EngineError {
    EngineError::Msg(Message::new("error-command-bad-arg").with_str("arg", name))
}

fn not_found(id: DownloadId) -> EngineError {
    EngineError::Msg(Message::new("error-engine-not-found").with_str("id", id.to_string()))
}

fn queue_not_found(q: &str) -> EngineError {
    EngineError::Msg(Message::new("error-engine-queue-not-found").with_str("queue", q))
}

fn required_id(args: &CommandArgs) -> EngineResult<DownloadId> {
    args.download_id("download_id")
        .ok_or_else(|| bad_arg("download_id"))
}

fn required_str<'a>(args: &'a CommandArgs, name: &str) -> EngineResult<&'a str> {
    args.str(name).ok_or_else(|| bad_arg(name))
}

fn required_uint(args: &CommandArgs, name: &str) -> EngineResult<u64> {
    args.uint(name).ok_or_else(|| bad_arg(name))
}

fn connections_arg(v: u64) -> EngineResult<u32> {
    u32::try_from(v)
        .ok()
        .filter(|c| (1..=limits::MAX_CONNECTIONS_PER_DOWNLOAD).contains(c))
        .ok_or_else(|| bad_arg("connections"))
}

fn speed_arg(v: u64) -> EngineResult<u64> {
    if v > limits::MAX_SPEED_LIMIT_BYTES_PER_SEC {
        return Err(bad_arg("bytes_per_sec"));
    }
    Ok(v)
}

fn valid_queue_id(q: &str) -> bool {
    !q.is_empty()
        && q.len() <= 64
        && q.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn ok() -> Value {
    json!({})
}

/// Everything needed to add one download.
#[derive(Debug, Default, Clone)]
pub struct AddSpec {
    /// URL (may carry credentials; they move to the keychain).
    pub url: String,
    /// Folder; `None` = category folder.
    pub dir: Option<PathBuf>,
    /// File name chosen by the user.
    pub file_name: Option<String>,
    /// Connections; `None` = settings default.
    pub connections: Option<u32>,
    /// Category; `None` = classify by name.
    pub category: Option<String>,
    /// Queue; `None` = main.
    pub queue: Option<String>,
    /// Start right away.
    pub start: bool,
    /// Expected checksum.
    pub checksum: Option<String>,
    /// Referer.
    pub referer: Option<String>,
    /// Cookie header handed over by the browser extension.
    pub cookie: Option<Secret>,
    /// Speed limit.
    pub speed_limit: u64,
    /// HTTPS→HTTP downgrade allowed.
    pub allow_downgrade: bool,
}

impl Engine {
    /// Runs a command. Validates arguments against the registry first.
    pub async fn execute(&self, command: &str, args: CommandArgs) -> EngineResult<Value> {
        let spec = registry::find(command).ok_or_else(|| {
            EngineError::Msg(Message::new("error-command-unknown").with_str("command", command))
        })?;
        args.validate(spec)?;
        let out = self.dispatch(command, &args).await;
        self.pump();
        out
    }

    async fn dispatch(&self, command: &str, a: &CommandArgs) -> EngineResult<Value> {
        match command {
            "download.add" => {
                let spec = AddSpec {
                    url: required_str(a, "url")?.to_owned(),
                    dir: a.str("dir").map(PathBuf::from),
                    file_name: a.str("file_name").map(str::to_owned),
                    connections: a.uint("connections").map(connections_arg).transpose()?,
                    category: a.str("category").map(str::to_owned),
                    queue: a.str("queue").map(str::to_owned),
                    start: a.bool("start").unwrap_or(true),
                    checksum: a.str("checksum").map(str::to_owned),
                    referer: a.str("referer").map(str::to_owned),
                    cookie: None,
                    speed_limit: speed_arg(a.uint("speed_limit").unwrap_or(0))?,
                    allow_downgrade: a.bool("allow_downgrade").unwrap_or(false),
                };
                let id = self.add_download(spec)?;
                Ok(json!({ "download_id": id.to_string() }))
            }
            "download.pause" => self.pause(required_id(a)?).map(|()| ok()),
            "download.resume" => self.resume(required_id(a)?).map(|()| ok()),
            "download.remove" => self
                .remove(required_id(a)?, a.bool("delete_file").unwrap_or(false))
                .map(|()| ok()),
            "download.restart" => self.restart(required_id(a)?).map(|()| ok()),
            "download.verify" => self.verify(required_id(a)?, a.str("checksum")).await,
            "download.set_connections" => {
                let id = required_id(a)?;
                let c = connections_arg(required_uint(a, "connections")?)?;
                self.with_entry(id, |e| {
                    e.rec.connections = c;
                    if let Some(l) = &e.live {
                        l.connections.store(c, Ordering::Relaxed);
                    }
                    Ok(())
                })?;
                Ok(ok())
            }
            "download.pause_all" => {
                let ids =
                    self.ids_where(|e| e.live.is_some() || e.status == DownloadStatus::Queued);
                for id in ids {
                    // Ignore downloads that changed state meanwhile.
                    let _ = self.pause(id);
                }
                Ok(ok())
            }
            "download.resume_all" => {
                let ids = self.ids_where(|e| {
                    e.live.is_none()
                        && matches!(
                            e.status,
                            DownloadStatus::Paused(_) | DownloadStatus::Failed(_)
                        )
                });
                for id in ids {
                    let _ = self.resume(id);
                }
                Ok(ok())
            }
            "download.remove_completed" => {
                let ids = self.ids_where(|e| e.status == DownloadStatus::Completed);
                let n = ids.len();
                for id in ids {
                    let _ = self.remove(id, false);
                }
                Ok(json!({ "removed": n }))
            }
            "queue.start" => self
                .queue_run(a.str("queue").unwrap_or(MAIN_QUEUE), true)
                .map(|()| ok()),
            "queue.stop" => self
                .queue_run(a.str("queue").unwrap_or(MAIN_QUEUE), false)
                .map(|()| ok()),
            "queue.create" => {
                let q = required_str(a, "queue")?;
                if !valid_queue_id(q) {
                    return Err(bad_arg("queue"));
                }
                let concurrency = a
                    .uint("concurrency")
                    .map(|c| {
                        u32::try_from(c)
                            .ok()
                            .filter(|c| (1..=limits::MAX_CONCURRENT_DOWNLOADS).contains(c))
                    })
                    .unwrap_or(Some(1))
                    .ok_or_else(|| bad_arg("concurrency"))?;
                let name = a.str("name").unwrap_or(q).to_owned();
                let mut st = self.state();
                if st.queues.iter().any(|x| x.id == q) {
                    return Err(EngineError::Msg(
                        Message::new("error-engine-queue-exists").with_str("queue", q),
                    ));
                }
                if st.queues.len() >= limits::MAX_QUEUES {
                    return Err(err("error-engine-too-many-queues"));
                }
                st.queues.push(QueueRecord {
                    id: q.to_owned(),
                    name,
                    concurrency,
                    running: false,
                    schedule: Schedule::default(),
                    items: Vec::new(),
                });
                Self::mark_dirty(&mut st);
                Ok(ok())
            }
            "queue.delete" => {
                let q = required_str(a, "queue")?;
                if q == MAIN_QUEUE {
                    return Err(err("error-engine-queue-main"));
                }
                let mut st = self.state();
                let Some(pos) = st.queues.iter().position(|x| x.id == q) else {
                    return Err(queue_not_found(q));
                };
                let removed = st.queues.remove(pos);
                for e in st.entries.iter_mut().filter(|e| e.rec.queue == q) {
                    e.rec.queue = MAIN_QUEUE.to_owned();
                }
                if let Some(main) = st.queues.iter_mut().find(|x| x.id == MAIN_QUEUE) {
                    main.items.extend(removed.items);
                }
                Self::mark_dirty(&mut st);
                Ok(ok())
            }
            "queue.set_concurrency" => {
                let q = required_str(a, "queue")?;
                let c = u32::try_from(required_uint(a, "concurrency")?)
                    .ok()
                    .filter(|c| (1..=limits::MAX_CONCURRENT_DOWNLOADS).contains(c))
                    .ok_or_else(|| bad_arg("concurrency"))?;
                self.with_queue(q, |qr| {
                    qr.concurrency = c;
                    Ok(())
                })?;
                Ok(ok())
            }
            "queue.move_item" => {
                let id = required_id(a)?;
                let sid = id.to_string();
                let mut st = self.state();
                let Some(e) = st
                    .entries
                    .iter_mut()
                    .find(|e| e.rec.download_id() == Some(id))
                else {
                    return Err(not_found(id));
                };
                let target = a
                    .str("queue")
                    .map_or_else(|| e.rec.queue.clone(), str::to_owned);
                let old = e.rec.queue.clone();
                if !st.queues.iter().any(|q| q.id == target) {
                    return Err(queue_not_found(&target));
                }
                if let Some(e) = st
                    .entries
                    .iter_mut()
                    .find(|e| e.rec.download_id() == Some(id))
                {
                    e.rec.queue = target.clone();
                }
                if let Some(q) = st.queues.iter_mut().find(|q| q.id == old) {
                    q.items.retain(|i| i != &sid);
                }
                if let Some(q) = st.queues.iter_mut().find(|q| q.id == target) {
                    q.items.retain(|i| i != &sid);
                    let pos = a
                        .uint("position")
                        .and_then(|p| usize::try_from(p).ok())
                        .unwrap_or(q.items.len())
                        .min(q.items.len());
                    q.items.insert(pos, sid);
                }
                Self::mark_dirty(&mut st);
                Ok(ok())
            }
            "schedule.set" => {
                let q = a.str("queue").unwrap_or(MAIN_QUEUE);
                let start_at = a
                    .str("start_at")
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let stop_at = a
                    .str("stop_at")
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                for t in [&start_at, &stop_at].into_iter().flatten() {
                    parse_time_of_day(t)?;
                }
                let days = a.list("days").map(parse_days).transpose()?;
                let after = a
                    .str("after_done")
                    .map(|s| AfterDone::parse(s).ok_or_else(|| bad_arg("after_done")))
                    .transpose()?;
                self.with_queue(q, |qr| {
                    if a.get("start_at").is_some() {
                        qr.schedule.start_at = start_at;
                    }
                    if a.get("stop_at").is_some() {
                        qr.schedule.stop_at = stop_at;
                    }
                    if let Some(d) = days {
                        qr.schedule.days = d;
                    }
                    if let Some(x) = after {
                        qr.schedule.after_done = x;
                    }
                    Ok(())
                })?;
                Ok(ok())
            }
            "schedule.clear" => {
                self.with_queue(a.str("queue").unwrap_or(MAIN_QUEUE), |qr| {
                    qr.schedule = Schedule::default();
                    Ok(())
                })?;
                Ok(ok())
            }
            "limit.set_global" => {
                let v = speed_arg(required_uint(a, "bytes_per_sec")?)?;
                self.inner.global.set_rate(v);
                let mut st = self.state();
                st.settings.global_speed_limit = v;
                Self::mark_settings_dirty(&mut st);
                Ok(ok())
            }
            "limit.set_download" => {
                let id = required_id(a)?;
                let v = speed_arg(required_uint(a, "bytes_per_sec")?)?;
                self.with_entry(id, |e| {
                    e.rec.speed_limit = v;
                    if let Some(l) = &e.live {
                        l.limiter.set_rate(v);
                    }
                    Ok(())
                })?;
                Ok(ok())
            }
            "category.assign" => {
                let id = required_id(a)?;
                let c = required_str(a, "category")?.to_owned();
                if !self.state().settings.categories.iter().any(|x| x.id == c) {
                    return Err(EngineError::Msg(
                        Message::new("error-engine-category-not-found").with_str("category", c),
                    ));
                }
                self.with_entry(id, |e| {
                    e.rec.category = c;
                    Ok(())
                })?;
                Ok(ok())
            }
            "category.set_folder" | "category.set_extensions" => {
                let c = required_str(a, "category")?;
                let mut st = self.state();
                let Some(cat) = st.settings.categories.iter_mut().find(|x| x.id == c) else {
                    return Err(EngineError::Msg(
                        Message::new("error-engine-category-not-found").with_str("category", c),
                    ));
                };
                if command == "category.set_folder" {
                    let f = required_str(a, "folder")?;
                    cat.folder = if f.is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(f))
                    };
                } else {
                    let exts = a.list("extensions").ok_or_else(|| bad_arg("extensions"))?;
                    if exts.len() > limits::MAX_CATEGORY_EXTENSIONS {
                        return Err(bad_arg("extensions"));
                    }
                    cat.extensions = exts
                        .iter()
                        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
                        .filter(|e| !e.is_empty())
                        .collect();
                }
                Self::mark_settings_dirty(&mut st);
                Ok(ok())
            }
            "capture.clipboard_toggle" => {
                let mut st = self.state();
                let v = a.bool("enabled").unwrap_or(!st.settings.clipboard_capture);
                st.settings.clipboard_capture = v;
                // Do not capture what is already on the clipboard.
                st.clipboard_last = None;
                Self::mark_settings_dirty(&mut st);
                Ok(json!({ "enabled": v }))
            }
            "capture.clipboard_text" => {
                let text = required_str(a, "text")?;
                if text.len() > limits::MAX_CLIPBOARD_BYTES {
                    return Err(bad_arg("text"));
                }
                let exts = self.state().settings.capture_extensions.clone();
                let links: Vec<String> = udm_proto::extract_urls_from_text(text)
                    .into_iter()
                    .filter(|u| link_matches_extensions(u, &exts))
                    .collect();
                Ok(json!({ "links": links }))
            }
            "capture.browser" => self.capture_browser(a),
            "batch.import" | "batch.pattern" => {
                let urls = if command == "batch.import" {
                    let text = required_str(a, "text")?;
                    if text.len() > limits::MAX_URL_LIST_BYTES {
                        return Err(bad_arg("text"));
                    }
                    udm_proto::parse_url_list(text)?
                } else {
                    udm_proto::expand_batch_pattern(required_str(a, "pattern")?)?
                };
                let ids = self.add_many(&urls, a)?;
                Ok(json!({ "download_ids": ids }))
            }
            "grabber.start" => self.grabber_start(a),
            "grabber.add_results" => {
                let job = required_uint(a, "job")?;
                let files = {
                    let st = self.state();
                    let Some(j) = st.grab_jobs.iter().find(|j| j.id == job) else {
                        return Err(EngineError::Msg(
                            Message::new("error-engine-job-not-found").with_uint("job", job),
                        ));
                    };
                    j.files.clone()
                };
                let ids = self.add_many(&files, a)?;
                Ok(json!({ "download_ids": ids }))
            }
            "settings.set" => {
                self.settings_set(required_str(a, "name")?, required_str(a, "value")?)
            }
            "tls.allow_host" => {
                let host = required_str(a, "host")?.to_ascii_lowercase();
                let fp = required_str(a, "fingerprint")?
                    .to_ascii_lowercase()
                    .replace(':', "");
                if host.is_empty() || fp.len() != 64 || !fp.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(bad_arg("fingerprint"));
                }
                {
                    let mut st = self.state();
                    st.settings.tls_exceptions.retain(|t| t.host != host);
                    st.settings.tls_exceptions.push(TlsExceptionSetting {
                        host: host.clone(),
                        sha256_fingerprint: fp,
                    });
                    Self::mark_settings_dirty(&mut st);
                }
                tracing::warn!(%host, "TLS certificate exception granted by the user");
                self.rebuild_client()?;
                Ok(ok())
            }
            "tls.revoke_host" => {
                let host = required_str(a, "host")?.to_ascii_lowercase();
                {
                    let mut st = self.state();
                    st.settings.tls_exceptions.retain(|t| t.host != host);
                    Self::mark_settings_dirty(&mut st);
                }
                self.rebuild_client()?;
                Ok(ok())
            }
            "proxy.set" => {
                let kind =
                    ProxyKind::parse(required_str(a, "kind")?).ok_or_else(|| bad_arg("kind"))?;
                let host = a.str("host").unwrap_or("").to_owned();
                let port = a
                    .uint("port")
                    .map(u16::try_from)
                    .transpose()
                    .map_err(|_| bad_arg("port"))?;
                if kind != ProxyKind::None && (host.is_empty() || port.is_none_or(|p| p == 0)) {
                    return Err(bad_arg("host"));
                }
                let user = a.str("user").filter(|u| !u.is_empty()).map(str::to_owned);
                if let Some(p) = a.str("password") {
                    self.put_secret(PROXY_SECRET_REF, &Secret::new(p))?;
                } else if user.is_none() {
                    self.delete_secret(PROXY_SECRET_REF);
                }
                {
                    let mut st = self.state();
                    st.settings.proxy.kind = kind;
                    st.settings.proxy.host = host;
                    st.settings.proxy.port = port.unwrap_or(0);
                    st.settings.proxy.user = user;
                    Self::mark_settings_dirty(&mut st);
                }
                self.rebuild_client()?;
                Ok(ok())
            }
            "auth.set_site" => {
                let host = required_str(a, "host")?.to_ascii_lowercase();
                let user = required_str(a, "user")?.to_owned();
                if host.is_empty() || user.is_empty() {
                    return Err(bad_arg("host"));
                }
                let sa = SiteAuth {
                    host: host.clone(),
                    user,
                };
                self.put_secret(&sa.secret_ref(), &Secret::new(required_str(a, "password")?))?;
                let old: Vec<SiteAuth> = {
                    let mut st = self.state();
                    let old = st
                        .settings
                        .site_auth
                        .iter()
                        .filter(|s| s.host == host && *s != &sa)
                        .cloned()
                        .collect();
                    st.settings.site_auth.retain(|s| s.host != host);
                    st.settings.site_auth.push(sa);
                    Self::mark_settings_dirty(&mut st);
                    old
                };
                for o in old {
                    self.delete_secret(&o.secret_ref());
                }
                Ok(ok())
            }
            "auth.remove_site" => {
                let host = required_str(a, "host")?.to_ascii_lowercase();
                let old: Vec<SiteAuth> = {
                    let mut st = self.state();
                    let old = st
                        .settings
                        .site_auth
                        .iter()
                        .filter(|s| s.host == host)
                        .cloned()
                        .collect();
                    st.settings.site_auth.retain(|s| s.host != host);
                    Self::mark_settings_dirty(&mut st);
                    old
                };
                for o in old {
                    self.delete_secret(&o.secret_ref());
                }
                Ok(ok())
            }
            "history.clear" | "history.remove" => {
                let all = {
                    let mut st = self.state();
                    if command == "history.clear" {
                        st.history.clear();
                    } else {
                        let id = required_id(a)?.to_string();
                        st.history.retain(|h| h.id != id);
                    }
                    st.history.clone()
                };
                if let Some(log) = crate::task::lock(&self.inner.history_log).as_mut() {
                    log.rewrite(&all)?;
                }
                Ok(ok())
            }
            _ => Err(EngineError::Msg(
                Message::new("error-command-unknown").with_str("command", command),
            )),
        }
    }

    fn ids_where(&self, f: impl Fn(&Entry) -> bool) -> Vec<DownloadId> {
        self.state()
            .entries
            .iter()
            .filter(|e| e.removing.is_none() && f(e))
            .filter_map(|e| e.rec.download_id())
            .collect()
    }

    fn with_entry<R>(
        &self,
        id: DownloadId,
        f: impl FnOnce(&mut Entry) -> EngineResult<R>,
    ) -> EngineResult<R> {
        let mut st = self.state();
        let e = st
            .entries
            .iter_mut()
            .find(|e| e.rec.download_id() == Some(id) && e.removing.is_none())
            .ok_or_else(|| not_found(id))?;
        let r = f(e)?;
        e.rec.status = StoredStatus::from_status(&e.status);
        Self::mark_dirty(&mut st);
        Ok(r)
    }

    fn with_queue<R>(
        &self,
        q: &str,
        f: impl FnOnce(&mut QueueRecord) -> EngineResult<R>,
    ) -> EngineResult<R> {
        let mut st = self.state();
        let qr = st
            .queues
            .iter_mut()
            .find(|x| x.id == q)
            .ok_or_else(|| queue_not_found(q))?;
        let r = f(qr)?;
        Self::mark_dirty(&mut st);
        Ok(r)
    }

    /// Adds one download and returns its id.
    pub fn add_download(&self, spec: AddSpec) -> EngineResult<DownloadId> {
        let parsed = udm_proto::validate_download_url(&spec.url)?;
        let (url, creds) = udm_proto::split_credentials(&parsed);
        if let Some(r) = &spec.referer
            && (!udm_proto::header_value_ok(r) || r.len() > limits::MAX_URL_BYTES)
        {
            return Err(bad_arg("referer"));
        }
        if let Some(c) = &spec.checksum {
            ChecksumSpec::parse(c)?;
        }
        if let Some(c) = &spec.cookie
            && (c.expose().len() > limits::MAX_COOKIE_BYTES
                || !udm_proto::header_value_ok(c.expose()))
        {
            return Err(bad_arg("cookie"));
        }
        let requested_name = match &spec.file_name {
            Some(n) if !n.trim().is_empty() => Some(udm_proto::sanitize_filename(n)),
            _ => None,
        };
        let (id, sid) = {
            let mut st = self.state();
            if st.entries.len() >= limits::MAX_QUEUE_SIZE {
                return Err(err("error-engine-queue-full"));
            }
            let queue = spec.queue.clone().unwrap_or_else(|| MAIN_QUEUE.to_owned());
            if !st.queues.iter().any(|q| q.id == queue) {
                return Err(queue_not_found(&queue));
            }
            if let Some(c) = &spec.category
                && !st.settings.categories.iter().any(|x| &x.id == c)
            {
                return Err(EngineError::Msg(
                    Message::new("error-engine-category-not-found")
                        .with_str("category", c.as_str()),
                ));
            }
            let id = DownloadId::new(st.next_id);
            st.next_id = st
                .next_id
                .checked_add(1)
                .ok_or_else(|| err("error-engine-queue-full"))?;
            let sid = id.to_string();
            let rec = DownloadRecord {
                id: sid.clone(),
                url: url.to_string(),
                dir: spec.dir.clone().unwrap_or_default(),
                file_name: None,
                requested_name,
                category: spec.category.clone().unwrap_or_default(),
                queue: queue.clone(),
                connections: spec.connections.unwrap_or(st.settings.connections),
                speed_limit: spec.speed_limit,
                checksum: spec.checksum.clone(),
                referer: spec.referer.clone(),
                cookie_ref: spec
                    .cookie
                    .as_ref()
                    .map(|_| DownloadRecord::cookie_secret_ref(&sid)),
                auth_user: creds.as_ref().map(|(u, _)| u.clone()),
                allow_downgrade: spec.allow_downgrade,
                status: StoredStatus::Queued,
                size: None,
                downloaded: 0,
                resumable: false,
                manual_start: spec.start,
                added_at: chrono::Utc::now().timestamp(),
                completed_at: None,
                notice: None,
            };
            let status = if spec.start {
                DownloadStatus::Queued
            } else {
                DownloadStatus::Paused(PauseReason::User)
            };
            st.entries.push(Entry {
                rec: DownloadRecord {
                    status: StoredStatus::from_status(&status),
                    ..rec
                },
                status,
                live: None,
                removing: None,
            });
            if let Some(q) = st.queues.iter_mut().find(|q| q.id == queue) {
                q.items.push(sid.clone());
            }
            Self::mark_dirty(&mut st);
            (id, sid)
        };
        // Secrets go to the keychain, never into the queue file.
        if let Some(c) = &spec.cookie {
            self.put_secret(&DownloadRecord::cookie_secret_ref(&sid), c)?;
        }
        if let Some((_, p)) = creds {
            self.put_secret(&DownloadRecord::auth_ref(&sid), &Secret::new(p))?;
        }
        Ok(id)
    }

    fn add_many(&self, urls: &[String], a: &CommandArgs) -> EngineResult<Vec<String>> {
        if urls.len() > limits::MAX_BATCH_SIZE {
            return Err(err("error-engine-batch-too-large"));
        }
        let mut ids = Vec::new();
        for u in urls {
            let spec = AddSpec {
                url: u.clone(),
                dir: a.str("dir").map(PathBuf::from),
                queue: a.str("queue").map(str::to_owned),
                start: a.bool("start").unwrap_or(false),
                ..AddSpec::default()
            };
            match self.add_download(spec) {
                Ok(id) => ids.push(id.to_string()),
                Err(e) => {
                    let m = e.to_message();
                    if m.key == "error-engine-queue-full" {
                        return Err(e);
                    }
                    self.emit(EngineEvent::Notice {
                        id: None,
                        message: m,
                    });
                }
            }
        }
        Ok(ids)
    }

    fn capture_browser(&self, a: &CommandArgs) -> EngineResult<Value> {
        let url = required_str(a, "url")?;
        let file_name = a.str("file_name").map(str::to_owned);
        let (min, exts) = {
            let st = self.state();
            (
                st.settings.capture_min_size,
                st.settings.capture_extensions.clone(),
            )
        };
        if let Some(size) = a.uint("size")
            && size < min
        {
            return Ok(json!({ "captured": false, "reason": "size" }));
        }
        let name_matches = file_name
            .as_deref()
            .and_then(udm_proto::extension)
            .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(&e)));
        if !exts.is_empty() && !name_matches && !link_matches_extensions(url, &exts) {
            return Ok(json!({ "captured": false, "reason": "extension" }));
        }
        let cookie = a.str("cookie").filter(|c| !c.is_empty()).map(Secret::new);
        let id = self.add_download(AddSpec {
            url: url.to_owned(),
            file_name,
            start: a.bool("start").unwrap_or(true),
            referer: a.str("referer").map(str::to_owned),
            cookie,
            ..AddSpec::default()
        })?;
        Ok(json!({ "captured": true, "download_id": id.to_string() }))
    }

    /// Pauses a running or waiting download.
    pub fn pause(&self, id: DownloadId) -> EngineResult<()> {
        self.with_entry(id, |e| {
            if let Some(l) = &e.live {
                l.request_stop(StopKind::Pause(PauseReason::User));
                e.rec.manual_start = false;
                return Ok(());
            }
            match e.status {
                DownloadStatus::Queued | DownloadStatus::Scheduled => {
                    e.status = DownloadStatus::Paused(PauseReason::User);
                    e.rec.manual_start = false;
                    Ok(())
                }
                DownloadStatus::Paused(_) => Ok(()),
                _ => Err(err("error-engine-cannot-pause")),
            }
        })
    }

    /// Resumes (or starts) a download outside queue processing.
    pub fn resume(&self, id: DownloadId) -> EngineResult<()> {
        self.with_entry(id, |e| {
            if e.live.is_some() {
                return Ok(());
            }
            match e.status {
                DownloadStatus::Completed => Err(err("error-engine-already-completed")),
                _ => {
                    e.status = DownloadStatus::Queued;
                    e.rec.manual_start = true;
                    Ok(())
                }
            }
        })
    }

    fn part_and_final(e: &Entry) -> Option<(PathBuf, PathBuf)> {
        let n = e.rec.file_name.as_ref()?;
        if e.rec.dir.as_os_str().is_empty() {
            return None;
        }
        Some((
            e.rec.dir.join(format!("{n}{PART_SUFFIX}")),
            e.rec.dir.join(n),
        ))
    }

    /// Removes a download (and optionally its file).
    pub fn remove(&self, id: DownloadId, delete_file: bool) -> EngineResult<()> {
        let mut st = self.state();
        let Some(idx) = st
            .entries
            .iter()
            .position(|e| e.rec.download_id() == Some(id) && e.removing.is_none())
        else {
            return Err(not_found(id));
        };
        if let Some(e) = st.entries.get_mut(idx)
            && let Some(l) = &e.live
        {
            l.request_stop(StopKind::Remove { delete_file });
            e.removing = Some(delete_file);
            Self::mark_dirty(&mut st);
            return Ok(());
        }
        let e = st.entries.remove(idx);
        for q in &mut st.queues {
            q.items.retain(|i| i != &e.rec.id);
        }
        Self::mark_dirty(&mut st);
        drop(st);
        let saved_part = if delete_file {
            task::saved_part_path(self.data_dir(), id)
        } else {
            None
        };
        let _ = std::fs::remove_file(self.data_dir().state_file(id));
        if let Some(part) = saved_part {
            remove_part(&part);
        }
        if let Some((part, fin)) = Self::part_and_final(&e)
            && delete_file
        {
            remove_part(&part);
            if e.status == DownloadStatus::Completed {
                let _ = std::fs::remove_file(udm_sys::long_path(&fin));
            }
        }
        if let Some(r) = &e.rec.cookie_ref {
            self.delete_secret(r);
        }
        if e.rec.auth_user.is_some() {
            self.delete_secret(&DownloadRecord::auth_ref(&e.rec.id));
        }
        Ok(())
    }

    /// Discards progress and starts from zero.
    pub fn restart(&self, id: DownloadId) -> EngineResult<()> {
        let part = self.with_entry(id, |e| {
            if e.live.is_some() {
                return Err(err("error-engine-download-active"));
            }
            let part = Self::part_and_final(e).map(|(p, _)| p);
            e.rec.downloaded = 0;
            e.rec.completed_at = None;
            e.rec.notice = None;
            if e.status != DownloadStatus::Completed {
                // Keep the name so the same part file is replaced.
            } else {
                e.rec.file_name = None;
            }
            e.status = DownloadStatus::Queued;
            e.rec.manual_start = true;
            Ok(part)
        })?;
        let _ = std::fs::remove_file(self.data_dir().state_file(id));
        if let Some(p) = part {
            let _ = std::fs::remove_file(udm_sys::long_path(&p));
        }
        Ok(())
    }

    async fn verify(&self, id: DownloadId, checksum: Option<&str>) -> EngineResult<Value> {
        let (path, stored) = {
            let st = self.state();
            let e = st
                .entries
                .iter()
                .find(|e| e.rec.download_id() == Some(id))
                .ok_or_else(|| not_found(id))?;
            if e.status != DownloadStatus::Completed {
                return Err(err("error-engine-not-completed"));
            }
            let (_, fin) =
                Self::part_and_final(e).ok_or_else(|| err("error-engine-not-completed"))?;
            (fin, e.rec.checksum.clone())
        };
        let spec = match checksum.map(str::to_owned).or(stored) {
            Some(c) => Some(ChecksumSpec::parse(&c)?),
            None => None,
        };
        let algo = spec
            .as_ref()
            .map_or(udm_proto::ChecksumAlgo::Sha256, |s| s.algo);
        let p2 = path.clone();
        let digest = tokio::task::spawn_blocking(move || hash_file(&p2, algo))
            .await
            .map_err(|_| err("error-engine-internal"))??;
        let actual = udm_proto::to_hex(&digest);
        let matches = spec.as_ref().map(|s| s.matches(&digest));
        if matches == Some(false) {
            self.emit(EngineEvent::Notice {
                id: Some(id),
                message: Message::new("notice-verify-mismatch")
                    .with_str("name", path.display().to_string()),
            });
        }
        Ok(json!({ "algo": algo.name(), "actual": actual, "matches": matches }))
    }

    fn queue_run(&self, q: &str, run: bool) -> EngineResult<()> {
        let mut st = self.state();
        let qr = st
            .queues
            .iter_mut()
            .find(|x| x.id == q)
            .ok_or_else(|| queue_not_found(q))?;
        qr.running = run;
        if run {
            st.queue_done.insert(q.to_owned(), 0);
            // Downloads paused by the user stay paused; failed ones retry.
            for e in st
                .entries
                .iter_mut()
                .filter(|e| e.rec.queue == q && e.live.is_none())
            {
                if matches!(
                    e.status,
                    DownloadStatus::Failed(_) | DownloadStatus::Paused(PauseReason::Schedule)
                ) {
                    e.status = DownloadStatus::Queued;
                }
            }
        } else {
            for e in st
                .entries
                .iter_mut()
                .filter(|e| e.rec.queue == q && !e.rec.manual_start)
            {
                if let Some(l) = &e.live {
                    l.request_stop(StopKind::Requeue);
                }
            }
        }
        Self::mark_dirty(&mut st);
        Ok(())
    }

    fn grabber_start(&self, a: &CommandArgs) -> EngineResult<Value> {
        let url = udm_proto::validate_download_url(required_str(a, "url")?)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(bad_arg("url"));
        }
        let depth = a
            .uint("depth")
            .map_or(Ok(1), |d| u32::try_from(d).map_err(|_| bad_arg("depth")))?
            .min(limits::MAX_GRABBER_DEPTH);
        let extensions: Vec<String> = a
            .list("extensions")
            .unwrap_or(&[])
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        let job = {
            let mut st = self.state();
            let id = st.next_job;
            st.next_job = st.next_job.saturating_add(1);
            st.grab_jobs.push(GrabJob {
                id,
                url: url.to_string(),
                state: "running".to_owned(),
                pages: 0,
                files: Vec::new(),
                error: None,
            });
            // Keep a bounded number of finished jobs.
            while st.grab_jobs.len() > 16 {
                st.grab_jobs.remove(0);
            }
            id
        };
        let engine = self.clone();
        let client = self.client();
        let opts = GrabOptions {
            url,
            depth,
            extensions,
            same_host: a.bool("same_host").unwrap_or(true),
        };
        tokio::spawn(async move {
            let r = crawl(client, opts).await;
            let mut st = engine.state();
            if let Some(j) = st.grab_jobs.iter_mut().find(|j| j.id == job) {
                j.pages = r.pages;
                j.files = r.files;
                j.state = if r.error.is_some() { "failed" } else { "done" }.to_owned();
                j.error = r.error.as_ref().map(StoredMessage::from);
            }
        });
        Ok(json!({ "job": job }))
    }

    /// Grabber jobs.
    pub fn grab_jobs(&self) -> Vec<GrabJob> {
        self.state().grab_jobs.clone()
    }

    fn settings_set(&self, name: &str, value: &str) -> EngineResult<Value> {
        let parse_bool = |v: &str| match v {
            "true" | "1" | "on" => Ok(true),
            "false" | "0" | "off" => Ok(false),
            _ => Err(bad_arg("value")),
        };
        let parse_u64 = |v: &str| v.trim().parse::<u64>().map_err(|_| bad_arg("value"));
        let mut st = self.state();
        let mut s = st.settings.clone();
        let mut rebuild = false;
        match name {
            "locale" => s.locale = value.to_owned(),
            "download_dir" => {
                if value.trim().is_empty() || value.len() > limits::MAX_PATH_BYTES {
                    return Err(bad_arg("value"));
                }
                s.download_dir = PathBuf::from(value);
                if let Some(home) = udm_store::user_home() {
                    s.resolve_relative_dirs(&home);
                }
            }
            "category_subfolders" => s.category_subfolders = parse_bool(value)?,
            "connections" => {
                s.connections = u32::try_from(parse_u64(value)?).map_err(|_| bad_arg("value"))?;
            }
            "max_concurrent" => {
                s.max_concurrent =
                    u32::try_from(parse_u64(value)?).map_err(|_| bad_arg("value"))?;
            }
            "clipboard_capture" => {
                s.clipboard_capture = parse_bool(value)?;
                st.clipboard_last = None;
            }
            "notify_on_complete" => s.notify_on_complete = parse_bool(value)?,
            "close_to_tray" => s.close_to_tray = parse_bool(value)?,
            "capture_extensions" => {
                let list: Vec<String> = value
                    .split([',', ' '])
                    .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
                    .filter(|e| !e.is_empty())
                    .collect();
                if list.len() > limits::MAX_ARG_LIST_ITEMS {
                    return Err(bad_arg("value"));
                }
                s.capture_extensions = list;
            }
            "capture_min_size" => s.capture_min_size = parse_u64(value)?,
            "user_agent" => {
                s.user_agent = value.to_owned();
                rebuild = true;
            }
            "global_speed_limit" => {
                s.global_speed_limit = speed_arg(parse_u64(value)?)?;
                self.inner.global.set_rate(s.global_speed_limit);
            }
            _ => {
                return Err(EngineError::Msg(
                    Message::new("error-settings-unknown").with_str("name", name),
                ));
            }
        }
        s.validate()?;
        st.settings = s;
        Self::mark_settings_dirty(&mut st);
        drop(st);
        if rebuild {
            self.rebuild_client()?;
        }
        Ok(ok())
    }
}

/// Convenience for front-ends: build [`CommandArgs`] from JSON.
pub fn args_from_json(v: &Value) -> Result<CommandArgs, Message> {
    use udm_core::ArgValue;
    let mut out = CommandArgs::new();
    let Some(obj) = v.as_object() else {
        if v.is_null() {
            return Ok(out);
        }
        return Err(Message::new("error-command-bad-args"));
    };
    if obj.len() > limits::MAX_COMMAND_ARGS {
        return Err(Message::new("error-command-too-many-args"));
    }
    for (k, val) in obj {
        let av = match val {
            Value::Null => ArgValue::Null,
            Value::Bool(b) => ArgValue::Bool(*b),
            Value::Number(n) => match n.as_u64() {
                Some(u) => ArgValue::UInt(u),
                None => {
                    return Err(Message::new("error-command-bad-arg").with_str("arg", k.as_str()));
                }
            },
            Value::String(s) => ArgValue::Str(s.clone()),
            Value::Array(items) => {
                if items.len() > limits::MAX_ARG_LIST_ITEMS {
                    return Err(Message::new("error-command-bad-arg").with_str("arg", k.as_str()));
                }
                let mut list = Vec::new();
                for i in items {
                    match i {
                        Value::String(s) => list.push(s.clone()),
                        Value::Number(n) => list.push(n.to_string()),
                        _ => {
                            return Err(
                                Message::new("error-command-bad-arg").with_str("arg", k.as_str())
                            );
                        }
                    }
                }
                ArgValue::List(list)
            }
            Value::Object(_) => {
                return Err(Message::new("error-command-bad-arg").with_str("arg", k.as_str()));
            }
        };
        out.set(k, av);
    }
    Ok(out)
}

/// Deletes a part file; a missing file is fine, other failures are logged.
pub(crate) fn remove_part(part: &std::path::Path) {
    if let Err(e) = std::fs::remove_file(udm_sys::long_path(part))
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("cannot delete part file {}: {e}", part.display());
    }
}

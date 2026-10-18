//! The window: toolbar, sidebar, download table, details, dialogs and toasts.
//! Renders engine snapshots and dispatches commands; no business rules here.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Id, RichText, Sense, Ui, UserData, ViewportCommand};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc, oneshot};
use udm_core::{Message, limits};
use udm_engine::grabber::GrabJob;
use udm_engine::{
    DownloadView, Engine, EngineEvent, MAIN_QUEUE, QueueRecord, Settings, Stats, args_from_json,
};
use udm_i18n::{
    Locale, Localizer, TrArg, format_bytes, format_duration, format_percent, format_speed,
};

use crate::bridge::{Bridge, UiRequest};
use crate::shot;
use crate::tray::{Tray, TrayAction};

const TOAST_SECS: u64 = 6;
const MAX_TOASTS: usize = 5;
const REPAINT_MS: u64 = 250;
const MAX_LISTED_LINKS: usize = 200;
const KIB: u64 = 1024;
const NAME_COL_WIDTH: f32 = 240.0;
const AFTER_DONE: &[&str] = &["none", "exit", "shutdown", "sleep", "hibernate"];

/// Which downloads the table shows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Filter {
    All,
    Unfinished,
    Finished,
    Category(String),
    Queue(String),
}

struct Toast {
    text: String,
    error: bool,
    at: Instant,
}

struct CmdResult {
    cmd: &'static str,
    result: Result<Value, Message>,
}

struct PendingShot {
    id: u64,
    path: std::path::PathBuf,
    reply: oneshot::Sender<Result<Value, Message>>,
    at: Instant,
}

struct AddForm {
    url: String,
    dir: String,
    file_name: String,
    connections: u32,
    category: String,
    start: bool,
    checksum: String,
}

struct SettingsForm {
    locale: Locale,
    download_dir: String,
    connections: u32,
    max_concurrent: u32,
    limit_kib: u64,
    clipboard: bool,
    notify: bool,
    close_to_tray: bool,
    subfolders: bool,
}

struct ScheduleForm {
    queue: String,
    start_at: String,
    stop_at: String,
    days: String,
    after_done: &'static str,
}

struct GrabForm {
    url: String,
    depth: u32,
    extensions: String,
    same_host: bool,
    job: Option<u64>,
}

struct PropForm {
    id: String,
    connections: u32,
    limit_kib: u64,
    checksum: String,
    verify: Option<String>,
}

enum Dialog {
    Add(AddForm),
    Settings(SettingsForm),
    Schedule(ScheduleForm),
    Batch {
        text: String,
        start: bool,
    },
    Grabber(GrabForm),
    Remove {
        id: String,
        name: String,
        delete_file: bool,
    },
    Properties(PropForm),
    Captured(Vec<String>),
}

/// What a dialog asks for after a frame.
#[derive(Default)]
struct Outcome {
    close: bool,
    run: Vec<(&'static str, Value)>,
    locale: Option<Locale>,
}

/// The application state kept between frames.
pub struct UdmApp {
    engine: Engine,
    rt: Handle,
    loc: Localizer,
    ui_rx: mpsc::Receiver<UiRequest>,
    events_rx: std_mpsc::Receiver<EngineEvent>,
    results_tx: std_mpsc::Sender<CmdResult>,
    results_rx: std_mpsc::Receiver<CmdResult>,
    bridge: Arc<Bridge>,
    shots: Vec<PendingShot>,
    next_shot: u64,
    filter: Filter,
    selected: Option<String>,
    dialog: Option<Dialog>,
    toasts: Vec<Toast>,
    tray: Option<Tray>,
    /// Set when the app should really exit instead of hiding to the tray.
    quitting: bool,
    /// The "still running in the tray" notice was shown this session.
    hidden_notice: bool,
}

fn locale_from_settings(s: &Settings) -> Locale {
    Locale::parse(&s.locale).unwrap_or(Locale::Id)
}

fn locale_tag(l: Locale) -> &'static str {
    match l {
        Locale::Id => "id",
        Locale::En => "en",
    }
}

fn is_finished(d: &DownloadView) -> bool {
    d.status == "completed"
}

fn kib_of(bytes: u64) -> u64 {
    bytes.checked_div(KIB).unwrap_or(0)
}

impl UdmApp {
    /// Builds the app and starts forwarding engine events to the GUI.
    pub fn new(
        engine: Engine,
        rt: Handle,
        ui_rx: mpsc::Receiver<UiRequest>,
        bridge: Arc<Bridge>,
        startup: Vec<Message>,
    ) -> Self {
        let loc = Localizer::new(locale_from_settings(&engine.settings()));
        let (events_tx, events_rx) = std_mpsc::channel();
        let mut sub = engine.subscribe();
        let wake = Arc::clone(&bridge);
        rt.spawn(async move {
            loop {
                match sub.recv().await {
                    Ok(ev) => {
                        if events_tx.send(ev).is_err() {
                            return;
                        }
                        wake.wake();
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        let (results_tx, results_rx) = std_mpsc::channel();
        let mut app = Self {
            engine,
            rt,
            loc,
            ui_rx,
            events_rx,
            results_tx,
            results_rx,
            bridge,
            shots: Vec::new(),
            next_shot: 1,
            filter: Filter::All,
            selected: None,
            dialog: None,
            toasts: Vec::new(),
            tray: None,
            quitting: false,
            hidden_notice: false,
        };
        for m in startup {
            app.toast_msg(&m, true);
        }
        app
    }

    /// Attaches the system tray icon (created on the UI thread).
    pub fn attach_tray(&mut self, tray: Option<Tray>) {
        self.tray = tray;
    }

    /// Active UI locale.
    pub fn locale(&self) -> Locale {
        self.loc.locale()
    }

    fn tr(&self, key: &str) -> String {
        self.loc.tr(key)
    }

    fn toast(&mut self, text: String, error: bool) {
        if self.toasts.len() >= MAX_TOASTS {
            self.toasts.remove(0);
        }
        self.toasts.push(Toast {
            text,
            error,
            at: Instant::now(),
        });
    }

    fn toast_msg(&mut self, m: &Message, error: bool) {
        let text = self.loc.message(m);
        self.toast(text, error);
    }

    /// Dispatches a command on the runtime; the result comes back as a [`CmdResult`].
    fn run(&self, cmd: &'static str, args: Value) {
        let engine = self.engine.clone();
        let tx = self.results_tx.clone();
        let wake = Arc::clone(&self.bridge);
        self.rt.spawn(async move {
            let result = match args_from_json(&args) {
                Ok(a) => engine.execute(cmd, a).await.map_err(|e| e.to_message()),
                Err(m) => Err(m),
            };
            let _ = tx.send(CmdResult { cmd, result });
            wake.wake();
        });
    }

    fn set_locale(&mut self, ctx: &egui::Context, l: Locale) {
        self.loc = Localizer::new(l);
        ctx.send_viewport_cmd(ViewportCommand::Title(self.tr("app-title")));
        if let Some(t) = &self.tray {
            t.relabel(&self.loc);
        }
        self.run(
            "settings.set",
            json!({ "name": "locale", "value": locale_tag(l) }),
        );
    }

    // ---- polling ----

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(req) = self.ui_rx.try_recv() {
            self.ui_request(ctx, req);
        }
        let shots: Vec<(u64, Arc<egui::ColorImage>)> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot {
                        user_data, image, ..
                    } => user_data
                        .data
                        .as_ref()
                        .and_then(|d| d.downcast_ref::<u64>())
                        .map(|id| (*id, Arc::clone(image))),
                    _ => None,
                })
                .collect()
        });
        for (id, image) in shots {
            if let Some(pos) = self.shots.iter().position(|s| s.id == id) {
                let s = self.shots.swap_remove(pos);
                let r = shot::save(&s.path, &image).map(|bytes| {
                    json!({
                        "path": s.path.display().to_string(),
                        "bytes": bytes,
                        "width": image.size[0],
                        "height": image.size[1],
                    })
                });
                let _ = s.reply.send(r);
            }
        }
        let stale = Duration::from_secs(limits::UI_REPLY_TIMEOUT_SECS);
        self.shots.retain(|s| s.at.elapsed() < stale);

        while let Ok(ev) = self.events_rx.try_recv() {
            self.engine_event(ctx, ev);
        }
        while let Ok(r) = self.results_rx.try_recv() {
            self.cmd_result(r);
        }
        self.toasts
            .retain(|t| t.at.elapsed() < Duration::from_secs(TOAST_SECS));
    }

    fn ui_request(&mut self, ctx: &egui::Context, req: UiRequest) {
        let param = |name: &str| {
            req.params
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let result = match req.method.as_str() {
            "ui.set_locale" => match param("locale").as_deref().and_then(Locale::parse) {
                Some(l) => {
                    self.set_locale(ctx, l);
                    Ok(json!({ "locale": locale_tag(l) }))
                }
                None => Err(Message::new("error-ui-bad-locale")
                    .with_str("locale", param("locale").unwrap_or_default())),
            },
            "ui.screenshot" => {
                let raw = param("path").unwrap_or_else(|| "target/shots/udm.png".to_owned());
                match shot::check_path(&raw) {
                    Ok(path) => {
                        let id = self.next_shot;
                        self.next_shot = self.next_shot.saturating_add(1);
                        ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::new(id)));
                        ctx.request_repaint();
                        self.shots.push(PendingShot {
                            id,
                            path,
                            reply: req.reply,
                            at: Instant::now(),
                        });
                        return;
                    }
                    Err(m) => Err(m),
                }
            }
            "ui.open" => {
                let name = param("dialog").unwrap_or_default();
                if self.open_dialog(&name) {
                    Ok(json!({ "dialog": name }))
                } else {
                    Err(Message::new("error-ui-unknown-dialog").with_str("dialog", name))
                }
            }
            "ui.close" => {
                self.dialog = None;
                Ok(json!({}))
            }
            "ui.select" => {
                let id = param("download_id");
                let known = id
                    .as_ref()
                    .is_some_and(|id| self.engine.downloads().iter().any(|d| &d.id == id));
                if known {
                    self.selected = id;
                    Ok(json!({}))
                } else {
                    Err(Message::new("error-control-missing-param")
                        .with_str("param", "download_id"))
                }
            }
            m => Err(Message::new("error-control-unknown-method").with_str("method", m)),
        };
        ctx.request_repaint();
        let _ = req.reply.send(result);
    }

    fn engine_event(&mut self, ctx: &egui::Context, ev: EngineEvent) {
        match ev {
            EngineEvent::Completed { file_name, .. } => {
                let text = self.loc.tr_args(
                    "ui-notice-completed",
                    &[("name", TrArg::Str(file_name.as_str().into()))],
                );
                if self.engine.settings().notify_on_complete {
                    let title = self.tr("app-title");
                    let body = text.clone();
                    self.rt.spawn_blocking(move || {
                        if let Err(e) = udm_sys::notify(&title, &body) {
                            tracing::debug!("notification failed: {e:?}");
                        }
                    });
                }
                self.toast(text, false);
            }
            EngineEvent::Failed { message, .. } => self.toast_msg(&message, true),
            EngineEvent::Notice { message, .. } => self.toast_msg(&message, false),
            EngineEvent::ExitRequested => {
                self.quitting = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            EngineEvent::ClipboardLinks(links) => {
                if self.dialog.is_none() && !links.is_empty() {
                    self.dialog = Some(Dialog::Captured(links));
                }
            }
        }
    }

    fn cmd_result(&mut self, r: CmdResult) {
        match (r.cmd, r.result) {
            (_, Err(m)) => self.toast_msg(&m, true),
            ("grabber.start", Ok(v)) => {
                if let Some(Dialog::Grabber(g)) = &mut self.dialog {
                    g.job = v.get("job").and_then(Value::as_u64);
                }
            }
            ("download.verify", Ok(v)) => {
                let algo = v.get("algo").and_then(Value::as_str).unwrap_or_default();
                let actual = v.get("actual").and_then(Value::as_str).unwrap_or_default();
                let key = match v.get("matches").and_then(Value::as_bool) {
                    Some(true) => "ui-verify-match",
                    Some(false) => "ui-verify-mismatch",
                    None => "ui-verify-result",
                };
                let text = self.loc.tr_args(
                    key,
                    &[
                        ("algo", TrArg::Str(algo.into())),
                        ("hash", TrArg::Str(actual.into())),
                    ],
                );
                if let Some(Dialog::Properties(p)) = &mut self.dialog {
                    p.verify = Some(text.clone());
                }
                self.toast(text, false);
            }
            _ => {}
        }
    }

    // ---- dialogs ----

    fn open_dialog(&mut self, name: &str) -> bool {
        let s = self.engine.settings();
        let selected = self
            .selected_view()
            .or_else(|| self.engine.downloads().into_iter().next());
        let queue = match &self.filter {
            Filter::Queue(q) => q.clone(),
            _ => MAIN_QUEUE.to_owned(),
        };
        self.dialog = match name {
            "add" => Some(Dialog::Add(AddForm {
                url: String::new(),
                dir: String::new(),
                file_name: String::new(),
                connections: s.connections,
                category: String::new(),
                start: true,
                checksum: String::new(),
            })),
            "settings" => Some(Dialog::Settings(SettingsForm {
                locale: self.loc.locale(),
                download_dir: s.download_dir.display().to_string(),
                connections: s.connections,
                max_concurrent: s.max_concurrent,
                limit_kib: kib_of(s.global_speed_limit),
                clipboard: s.clipboard_capture,
                notify: s.notify_on_complete,
                close_to_tray: s.close_to_tray,
                subfolders: s.category_subfolders,
            })),
            "schedule" => {
                let q = self.engine.queues().into_iter().find(|x| x.id == queue);
                let sched = q.map(|q| q.schedule).unwrap_or_default();
                let after = AFTER_DONE
                    .iter()
                    .copied()
                    .find(|a| *a == sched.after_done.as_str())
                    .unwrap_or("none");
                let days = sched
                    .days
                    .iter()
                    .filter_map(|d| {
                        ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].get(usize::from(*d))
                    })
                    .copied()
                    .collect::<Vec<_>>()
                    .join(",");
                Some(Dialog::Schedule(ScheduleForm {
                    queue,
                    start_at: sched.start_at.unwrap_or_default(),
                    stop_at: sched.stop_at.unwrap_or_default(),
                    days,
                    after_done: after,
                }))
            }
            "batch" => Some(Dialog::Batch {
                text: String::new(),
                start: true,
            }),
            "grabber" => Some(Dialog::Grabber(GrabForm {
                url: String::new(),
                depth: 1,
                extensions: String::new(),
                same_host: true,
                job: None,
            })),
            "properties" => {
                let Some(d) = selected else { return false };
                self.selected = Some(d.id.clone());
                Some(Dialog::Properties(PropForm {
                    id: d.id,
                    connections: d.connections,
                    limit_kib: kib_of(d.speed_limit),
                    checksum: d.checksum.unwrap_or_default(),
                    verify: None,
                }))
            }
            "remove" => {
                let Some(d) = selected else { return false };
                self.selected = Some(d.id.clone());
                Some(Dialog::Remove {
                    id: d.id,
                    name: d.file_name,
                    delete_file: false,
                })
            }
            "captured" => Some(Dialog::Captured(vec![
                "http://127.0.0.1/example.zip".to_owned(),
            ])),
            _ => return false,
        };
        true
    }

    fn selected_view(&self) -> Option<DownloadView> {
        let id = self.selected.as_ref()?;
        self.engine.downloads().into_iter().find(|d| &d.id == id)
    }

    fn draw_dialog(&mut self, ctx: &egui::Context, settings: &Settings, queues: &[QueueRecord]) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let jobs = self.engine.grab_jobs();
        let downloads = self.engine.downloads();
        let (title_key, id) = match &dialog {
            Dialog::Add(_) => ("ui-add-title", "dlg-add"),
            Dialog::Settings(_) => ("ui-settings-title", "dlg-settings"),
            Dialog::Schedule(_) => ("cmd-schedule-set", "dlg-schedule"),
            Dialog::Batch { .. } => ("cmd-batch-import", "dlg-batch"),
            Dialog::Grabber(_) => ("cmd-grabber-start", "dlg-grabber"),
            Dialog::Remove { .. } => ("ui-remove-title", "dlg-remove"),
            Dialog::Properties(_) => ("ui-properties-title", "dlg-properties"),
            Dialog::Captured(_) => ("ui-captured-title", "dlg-captured"),
        };
        let mut open = true;
        let mut out = Outcome::default();
        let loc = &self.loc;
        egui::Window::new(loc.tr(title_key))
            .id(Id::new(id))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                out = match &mut dialog {
                    Dialog::Add(f) => add_dialog(ui, loc, f, settings, queues),
                    Dialog::Settings(f) => settings_dialog(ui, loc, f),
                    Dialog::Schedule(f) => schedule_dialog(ui, loc, f, queues),
                    Dialog::Batch { text, start } => batch_dialog(ui, loc, text, start),
                    Dialog::Grabber(f) => grabber_dialog(ui, loc, f, &jobs),
                    Dialog::Remove {
                        id,
                        name,
                        delete_file,
                    } => remove_dialog(ui, loc, id, name, delete_file),
                    Dialog::Properties(f) => properties_dialog(ui, loc, f, &downloads),
                    Dialog::Captured(links) => captured_dialog(ui, loc, links),
                };
            });
        if let Some(l) = out.locale {
            self.set_locale(ctx, l);
        }
        for (cmd, args) in out.run {
            self.run(cmd, args);
        }
        if open && !out.close {
            self.dialog = Some(dialog);
        }
    }

    // ---- main layout ----

    fn draw(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let downloads = self.engine.downloads();
        let stats = self.engine.stats();
        let settings = self.engine.settings();
        let queues = self.engine.queues();
        if let Some(id) = &self.selected
            && !downloads.iter().any(|d| &d.id == id)
        {
            self.selected = None;
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui, &settings);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("statusbar").show(ui, |ui| self.status_bar(ui, &stats));
        egui::Panel::bottom("details")
            .resizable(true)
            .default_size(170.0)
            .show(ui, |ui| self.details(ui, &downloads));
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| self.sidebar(ui, &downloads, &settings, &queues));
        egui::CentralPanel::default_margins().show(ui, |ui| self.table(ui, &downloads));

        self.draw_dialog(&ctx, &settings, &queues);
        self.draw_toasts(&ctx);
    }

    fn toolbar(&mut self, ui: &mut Ui, settings: &Settings) {
        let sel = self.selected_view();
        let queue = match &self.filter {
            Filter::Queue(q) => q.clone(),
            _ => MAIN_QUEUE.to_owned(),
        };
        ui.horizontal_wrapped(|ui| {
            if ui.button(self.tr("cmd-download-add")).clicked() {
                self.open_dialog("add");
            }
            let can_resume = sel
                .as_ref()
                .is_some_and(|d| matches!(d.status.as_str(), "paused" | "failed" | "queued"));
            let can_pause = sel
                .as_ref()
                .is_some_and(|d| matches!(d.status.as_str(), "downloading" | "probing" | "queued"));
            if ui
                .add_enabled(
                    can_resume,
                    egui::Button::new(self.tr("cmd-download-resume")),
                )
                .clicked()
                && let Some(d) = &sel
            {
                self.run("download.resume", json!({ "download_id": d.id }));
            }
            if ui
                .add_enabled(can_pause, egui::Button::new(self.tr("cmd-download-pause")))
                .clicked()
                && let Some(d) = &sel
            {
                self.run("download.pause", json!({ "download_id": d.id }));
            }
            if ui
                .add_enabled(
                    sel.is_some(),
                    egui::Button::new(self.tr("cmd-download-remove")),
                )
                .clicked()
            {
                self.open_dialog("remove");
            }
            ui.separator();
            if ui.button(self.tr("cmd-download-pause-all")).clicked() {
                self.run("download.pause_all", json!({}));
            }
            if ui.button(self.tr("cmd-download-resume-all")).clicked() {
                self.run("download.resume_all", json!({}));
            }
            ui.separator();
            if ui.button(self.tr("cmd-queue-start")).clicked() {
                self.run("queue.start", json!({ "queue": queue }));
            }
            if ui.button(self.tr("cmd-queue-stop")).clicked() {
                self.run("queue.stop", json!({ "queue": queue }));
            }
            if ui.button(self.tr("cmd-schedule-set")).clicked() {
                self.open_dialog("schedule");
            }
            ui.separator();
            if ui.button(self.tr("cmd-batch-import")).clicked() {
                self.open_dialog("batch");
            }
            if ui.button(self.tr("cmd-grabber-start")).clicked() {
                self.open_dialog("grabber");
            }
            if ui.button(self.tr("ui-settings-title")).clicked() {
                self.open_dialog("settings");
            }
            let mut clip = settings.clipboard_capture;
            if ui
                .checkbox(&mut clip, self.tr("cmd-capture-clipboard-toggle"))
                .changed()
            {
                self.run("capture.clipboard_toggle", json!({ "enabled": clip }));
            }
        });
    }

    fn sidebar(
        &mut self,
        ui: &mut Ui,
        downloads: &[DownloadView],
        settings: &Settings,
        queues: &[QueueRecord],
    ) {
        let count = |f: &dyn Fn(&DownloadView) -> bool| downloads.iter().filter(|d| f(d)).count();
        let label = |text: String, n: usize| format!("{text} ({n})");
        ui.add_space(4.0);
        egui::ScrollArea::vertical().show(ui, |ui| {
            let all = label(self.tr("ui-filter-all"), downloads.len());
            ui.selectable_value(&mut self.filter, Filter::All, all);
            let unfinished = label(self.tr("ui-filter-unfinished"), count(&|d| !is_finished(d)));
            ui.selectable_value(&mut self.filter, Filter::Unfinished, unfinished);
            let finished = label(self.tr("ui-filter-finished"), count(&is_finished));
            ui.selectable_value(&mut self.filter, Filter::Finished, finished);
            ui.add_space(6.0);
            ui.label(RichText::new(self.tr("ui-section-categories")).strong());
            for c in &settings.categories {
                let n = count(&|d| d.category == c.id);
                let text = label(self.tr(&format!("category-{}", c.id)), n);
                ui.selectable_value(&mut self.filter, Filter::Category(c.id.clone()), text);
            }
            ui.add_space(6.0);
            ui.label(RichText::new(self.tr("ui-section-queues")).strong());
            for q in queues {
                let name = if q.id == MAIN_QUEUE {
                    self.tr("ui-queue-main")
                } else if q.name.is_empty() {
                    q.id.clone()
                } else {
                    q.name.clone()
                };
                let mut text = label(name, q.items.len());
                if q.running {
                    text = format!("▶ {text}");
                } else if q.schedule.start_at.is_some() {
                    text = format!("⏰ {text}");
                }
                ui.selectable_value(&mut self.filter, Filter::Queue(q.id.clone()), text);
            }
        });
    }

    fn visible<'a>(&self, downloads: &'a [DownloadView]) -> Vec<&'a DownloadView> {
        downloads
            .iter()
            .filter(|d| match &self.filter {
                Filter::All => true,
                Filter::Unfinished => !is_finished(d),
                Filter::Finished => is_finished(d),
                Filter::Category(c) => &d.category == c,
                Filter::Queue(q) => &d.queue == q,
            })
            .collect()
    }

    fn table(&mut self, ui: &mut Ui, downloads: &[DownloadView]) {
        let rows = self.visible(downloads);
        if rows.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(self.tr("ui-empty-list")).weak())
            });
            return;
        }
        let l = self.loc.locale();
        let mut open: Option<&'static str> = None;
        let mut run: Vec<(&'static str, Value)> = Vec::new();
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("downloads")
                    .striped(true)
                    .num_columns(7)
                    .spacing([14.0, 6.0])
                    .show(ui, |ui| {
                        for key in [
                            "ui-col-name",
                            "ui-col-size",
                            "ui-col-progress",
                            "ui-col-speed",
                            "ui-col-eta",
                            "ui-col-status",
                            "ui-col-added",
                        ] {
                            ui.label(RichText::new(self.tr(key)).strong());
                        }
                        ui.end_row();
                        for d in rows {
                            let sel = self.selected.as_deref() == Some(d.id.as_str());
                            let name = if d.dangerous_name {
                                format!("⚠ {}", d.file_name)
                            } else {
                                d.file_name.clone()
                            };
                            let resp = ui
                                .allocate_ui_with_layout(
                                    egui::vec2(NAME_COL_WIDTH, 20.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| ui.add(egui::Button::selectable(sel, name).truncate()),
                                )
                                .inner;
                            if resp.clicked() {
                                self.selected = Some(d.id.clone());
                            }
                            if resp.double_clicked() {
                                self.selected = Some(d.id.clone());
                                open = Some("properties");
                            }
                            resp.context_menu(|ui| {
                                self.selected = Some(d.id.clone());
                                let done = d.status == "completed";
                                for (cmd, key, enabled) in [
                                    ("download.open", "cmd-download-open", done),
                                    ("download.open_folder", "cmd-download-open-folder", true),
                                ] {
                                    if ui
                                        .add_enabled(enabled, egui::Button::new(self.loc.tr(key)))
                                        .clicked()
                                    {
                                        run.push((cmd, json!({ "download_id": d.id })));
                                        ui.close();
                                    }
                                }
                                ui.separator();
                                for (cmd, key) in [
                                    ("download.resume", "cmd-download-resume"),
                                    ("download.pause", "cmd-download-pause"),
                                    ("download.restart", "cmd-download-restart"),
                                    ("download.verify", "cmd-download-verify"),
                                ] {
                                    if ui.button(self.loc.tr(key)).clicked() {
                                        let mut args = json!({ "download_id": d.id });
                                        if cmd == "download.restart"
                                            && let Some(a) = args.as_object_mut()
                                        {
                                            a.insert("confirm".into(), Value::Bool(true));
                                        }
                                        run.push((cmd, args));
                                        ui.close();
                                    }
                                }
                                if ui.button(self.loc.tr("cmd-download-remove")).clicked() {
                                    open = Some("remove");
                                    ui.close();
                                }
                                if ui.button(self.loc.tr("ui-properties-title")).clicked() {
                                    open = Some("properties");
                                    ui.close();
                                }
                            });
                            ui.label(match d.size {
                                Some(s) => format_bytes(l, s),
                                None => self.tr("ui-unknown-size"),
                            });
                            let (frac, pct) = match d.size {
                                Some(s) if s > 0 => (
                                    (d.downloaded as f32 / s as f32).clamp(0.0, 1.0),
                                    format_percent(l, d.downloaded, s),
                                ),
                                _ => (0.0, format_bytes(l, d.downloaded)),
                            };
                            ui.add(egui::ProgressBar::new(frac).desired_width(150.0).text(pct));
                            ui.label(if d.speed > 0 {
                                format_speed(l, d.speed)
                            } else {
                                String::new()
                            });
                            ui.label(match (d.status.as_str(), d.eta_secs) {
                                ("downloading", Some(e)) => format_duration(l, e),
                                _ => String::new(),
                            });
                            let status = self.tr(&d.status_key);
                            let status = match d.status.as_str() {
                                "failed" => {
                                    RichText::new(status).color(ui.visuals().error_fg_color)
                                }
                                "completed" => {
                                    RichText::new(status).color(Color32::from_rgb(40, 150, 70))
                                }
                                _ => RichText::new(status),
                            };
                            ui.label(status);
                            ui.label(local_time(d.added_at));
                            ui.end_row();
                        }
                    });
            });
        for (cmd, args) in run {
            self.run(cmd, args);
        }
        if let Some(name) = open {
            self.open_dialog(name);
        }
    }

    fn details(&mut self, ui: &mut Ui, downloads: &[DownloadView]) {
        let Some(d) = self
            .selected
            .as_ref()
            .and_then(|id| downloads.iter().find(|d| &d.id == id))
        else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(self.tr("ui-no-selection")).weak())
            });
            return;
        };
        let l = self.loc.locale();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&d.file_name).strong().size(15.0));
            ui.label(RichText::new(self.tr(&d.status_key)).weak());
        });
        if d.dangerous_name {
            let w = self.loc.tr_args(
                "ui-dangerous-name",
                &[("name", TrArg::Str(d.file_name.as_str().into()))],
            );
            ui.label(RichText::new(w).color(ui.visuals().warn_fg_color));
        }
        if let Some(m) = &d.message {
            ui.label(
                RichText::new(self.loc.message(&m.to_message())).color(ui.visuals().warn_fg_color),
            );
        }
        segment_bar(ui, d);
        egui::Grid::new("details-grid")
            .num_columns(4)
            .spacing([12.0, 3.0])
            .show(ui, |ui| {
                ui.label(self.tr("ui-detail-url"));
                ui.label(&d.url);
                ui.label(self.tr("ui-detail-connections"));
                ui.label(format!("{} / {}", d.active_connections, d.connections));
                ui.end_row();
                ui.label(self.tr("ui-detail-folder"));
                ui.label(&d.dir);
                ui.label(self.tr("ui-detail-limit"));
                ui.label(if d.speed_limit > 0 {
                    format_speed(l, d.speed_limit)
                } else {
                    self.tr("ui-limit-none")
                });
                ui.end_row();
                ui.label(self.tr("ui-detail-downloaded"));
                ui.label(format_bytes(l, d.downloaded));
                ui.label(self.tr("ui-detail-resume"));
                ui.label(self.tr(if d.resumable {
                    "ui-resume-yes"
                } else {
                    "ui-resume-no"
                }));
                ui.end_row();
            });
    }

    fn status_bar(&self, ui: &mut Ui, stats: &Stats) {
        let l = self.loc.locale();
        ui.horizontal(|ui| {
            ui.label(self.loc.tr_args(
                "ui-status-bar",
                &[
                    ("active", TrArg::Num(stats.active)),
                    ("total", TrArg::Num(stats.total)),
                    ("speed", TrArg::Str(format_speed(l, stats.speed).into())),
                ],
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(self.tr("app-title")).weak());
                ui.separator();
                ui.label(if stats.global_speed_limit > 0 {
                    self.loc.tr_args(
                        "ui-limit-global",
                        &[(
                            "limit",
                            TrArg::Str(format_speed(l, stats.global_speed_limit).into()),
                        )],
                    )
                } else {
                    self.tr("ui-limit-global-none")
                });
            });
        });
    }

    fn draw_toasts(&self, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(Id::new("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, [-12.0, -40.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for t in &self.toasts {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(360.0);
                        let text = RichText::new(&t.text);
                        ui.label(if t.error {
                            text.color(ui.visuals().error_fg_color)
                        } else {
                            text
                        });
                    });
                    ui.add_space(4.0);
                }
            });
    }
}

impl UdmApp {
    /// Tray actions, tooltip, and hiding to the tray instead of closing.
    fn tray_tick(&mut self, ctx: &egui::Context) {
        let Some(tray) = &mut self.tray else {
            return;
        };
        let actions = tray.actions();
        let stats = self.engine.stats();
        let status = self.loc.tr_args(
            "ui-status-bar",
            &[
                ("active", TrArg::Num(stats.active)),
                ("total", TrArg::Num(stats.total)),
                (
                    "speed",
                    TrArg::Str(format_speed(self.loc.locale(), stats.speed).into()),
                ),
            ],
        );
        tray.set_tooltip(format!(
            "{}
{status}",
            self.loc.tr("app-title")
        ));
        for a in actions {
            match a {
                TrayAction::Show => {
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                TrayAction::PauseAll => self.run("download.pause_all", json!({})),
                TrayAction::ResumeAll => self.run("download.resume_all", json!({})),
                TrayAction::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
        let close = ctx.input(|i| i.viewport().close_requested());
        if close && !self.quitting && self.engine.settings().close_to_tray {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            if !self.hidden_notice {
                self.hidden_notice = true;
                let title = self.tr("app-title");
                let body = self.tr("ui-tray-still-running");
                self.rt.spawn_blocking(move || {
                    if let Err(e) = udm_sys::notify(&title, &body) {
                        tracing::debug!("notification failed: {e:?}");
                    }
                });
            }
        }
    }
}

impl eframe::App for UdmApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.tray_tick(ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        // Panic guard: a bug in one frame is reported, the app keeps running.
        if catch_unwind(AssertUnwindSafe(|| self.draw(ui))).is_err() {
            let text = self.tr("error-ui-frame");
            self.dialog = None;
            self.toast(text, true);
        }
        ui.ctx()
            .request_repaint_after(Duration::from_millis(REPAINT_MS));
    }
}

fn local_time(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// Draws the segment map: done bytes filled, active segments with their
/// remaining part empty.
fn segment_bar(ui: &mut Ui, d: &DownloadView) {
    let width = ui.available_width().max(50.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 14.0), Sense::hover());
    let p = ui.painter();
    let empty = ui.visuals().extreme_bg_color;
    let done = Color32::from_rgb(52, 120, 220);
    let active = Color32::from_rgb(110, 175, 250);
    p.rect_filled(rect, 2.0, empty);
    let Some(size) = d.size.filter(|s| *s > 0) else {
        return;
    };
    let x = |off: u64| rect.left() + rect.width() * (off.min(size) as f32 / size as f32);
    if d.status == "completed" {
        p.rect_filled(rect, 2.0, done);
        return;
    }
    if d.segments.is_empty() {
        let r = egui::Rect::from_min_max(rect.min, egui::pos2(x(d.downloaded), rect.max.y));
        p.rect_filled(r, 2.0, done);
        return;
    }
    // Everything before the first segment and between segments is complete.
    let mut cursor = 0u64;
    let mut segs = d.segments.clone();
    segs.sort_by_key(|s| s.start);
    for s in &segs {
        if s.start > cursor {
            let r = egui::Rect::from_min_max(
                egui::pos2(x(cursor), rect.min.y),
                egui::pos2(x(s.start), rect.max.y),
            );
            p.rect_filled(r, 0.0, done);
        }
        let r = egui::Rect::from_min_max(
            egui::pos2(x(s.start), rect.min.y),
            egui::pos2(x(s.pos), rect.max.y),
        );
        p.rect_filled(r, 0.0, active);
        cursor = cursor.max(s.end);
    }
    if cursor < size {
        let r = egui::Rect::from_min_max(
            egui::pos2(x(cursor), rect.min.y),
            egui::pos2(x(size), rect.max.y),
        );
        p.rect_filled(r, 0.0, done);
    }
}

fn buttons(ui: &mut Ui, loc: &Localizer, ok_key: &str, ok_enabled: bool) -> (bool, bool) {
    let mut ok = false;
    let mut cancel = false;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ok = ui
            .add_enabled(ok_enabled, egui::Button::new(loc.tr(ok_key)))
            .clicked();
        cancel = ui.button(loc.tr("ui-btn-cancel")).clicked();
    });
    (ok, cancel)
}

fn text_row(ui: &mut Ui, label: String, value: &mut String, hint: Option<String>) {
    ui.label(label);
    let mut edit = egui::TextEdit::singleline(value).desired_width(380.0);
    if let Some(h) = hint {
        edit = edit.hint_text(h);
    }
    ui.add(edit);
    ui.end_row();
}

fn add_dialog(
    ui: &mut Ui,
    loc: &Localizer,
    f: &mut AddForm,
    settings: &Settings,
    queues: &[QueueRecord],
) -> Outcome {
    let _ = queues;
    egui::Grid::new("add-grid")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            text_row(
                ui,
                loc.tr("ui-add-url"),
                &mut f.url,
                Some("https://".to_owned()),
            );
            text_row(
                ui,
                loc.tr("ui-add-folder"),
                &mut f.dir,
                Some(loc.tr("ui-hint-auto")),
            );
            text_row(
                ui,
                loc.tr("ui-add-file-name"),
                &mut f.file_name,
                Some(loc.tr("ui-hint-auto")),
            );
            ui.label(loc.tr("ui-add-connections"));
            ui.add(
                egui::DragValue::new(&mut f.connections)
                    .range(1..=limits::MAX_CONNECTIONS_PER_DOWNLOAD),
            );
            ui.end_row();
            ui.label(loc.tr("ui-add-category"));
            let current = if f.category.is_empty() {
                loc.tr("ui-hint-auto")
            } else {
                loc.tr(&format!("category-{}", f.category))
            };
            egui::ComboBox::from_id_salt("add-category")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut f.category, String::new(), loc.tr("ui-hint-auto"));
                    for c in &settings.categories {
                        ui.selectable_value(
                            &mut f.category,
                            c.id.clone(),
                            loc.tr(&format!("category-{}", c.id)),
                        );
                    }
                });
            ui.end_row();
            text_row(
                ui,
                loc.tr("ui-add-checksum"),
                &mut f.checksum,
                Some("sha256:…".to_owned()),
            );
            ui.label("");
            ui.checkbox(&mut f.start, loc.tr("ui-add-start-now"));
            ui.end_row();
        });
    let (ok, cancel) = buttons(ui, loc, "ui-btn-add", !f.url.trim().is_empty());
    let mut out = Outcome {
        close: cancel,
        ..Default::default()
    };
    if ok {
        let mut args = serde_json::Map::new();
        args.insert("url".into(), Value::from(f.url.trim()));
        args.insert("connections".into(), Value::from(f.connections));
        args.insert("start".into(), Value::Bool(f.start));
        for (k, v) in [
            ("dir", &f.dir),
            ("file_name", &f.file_name),
            ("category", &f.category),
            ("checksum", &f.checksum),
        ] {
            if !v.trim().is_empty() {
                args.insert(k.into(), Value::from(v.trim()));
            }
        }
        out.run.push(("download.add", Value::Object(args)));
        out.close = true;
    }
    out
}

fn settings_dialog(ui: &mut Ui, loc: &Localizer, f: &mut SettingsForm) -> Outcome {
    egui::Grid::new("settings-grid")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(loc.tr("ui-settings-language"));
            egui::ComboBox::from_id_salt("settings-locale")
                .selected_text(loc.tr(if f.locale == Locale::Id {
                    "ui-lang-id"
                } else {
                    "ui-lang-en"
                }))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut f.locale, Locale::Id, loc.tr("ui-lang-id"));
                    ui.selectable_value(&mut f.locale, Locale::En, loc.tr("ui-lang-en"));
                });
            ui.end_row();
            text_row(
                ui,
                loc.tr("ui-settings-download-dir"),
                &mut f.download_dir,
                None,
            );
            ui.label(loc.tr("ui-add-connections"));
            ui.add(
                egui::DragValue::new(&mut f.connections)
                    .range(1..=limits::MAX_CONNECTIONS_PER_DOWNLOAD),
            );
            ui.end_row();
            ui.label(loc.tr("ui-settings-max-concurrent"));
            ui.add(egui::DragValue::new(&mut f.max_concurrent).range(1..=32));
            ui.end_row();
            ui.label(loc.tr("ui-settings-global-limit"));
            ui.add(
                egui::DragValue::new(&mut f.limit_kib)
                    .speed(16)
                    .suffix(loc.tr("ui-unit-kib-s")),
            );
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut f.subfolders, loc.tr("ui-settings-subfolders"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut f.clipboard, loc.tr("cmd-capture-clipboard-toggle"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut f.notify, loc.tr("ui-settings-notify"));
            ui.end_row();
            if cfg!(windows) {
                ui.label("");
                ui.checkbox(&mut f.close_to_tray, loc.tr("ui-settings-close-to-tray"));
                ui.end_row();
            }
        });
    let (ok, cancel) = buttons(ui, loc, "ui-btn-save", !f.download_dir.trim().is_empty());
    let mut out = Outcome {
        close: cancel,
        ..Default::default()
    };
    if ok {
        let set =
            |name: &str, value: String| ("settings.set", json!({ "name": name, "value": value }));
        out.run = vec![
            set("download_dir", f.download_dir.trim().to_owned()),
            set("connections", f.connections.to_string()),
            set("max_concurrent", f.max_concurrent.to_string()),
            set("category_subfolders", f.subfolders.to_string()),
            set("notify_on_complete", f.notify.to_string()),
            set("close_to_tray", f.close_to_tray.to_string()),
            (
                "capture.clipboard_toggle",
                json!({ "enabled": f.clipboard }),
            ),
            (
                "limit.set_global",
                json!({ "bytes_per_sec": f.limit_kib.saturating_mul(KIB) }),
            ),
        ];
        if f.locale != loc.locale() {
            out.locale = Some(f.locale);
        }
        out.close = true;
    }
    out
}

fn schedule_dialog(
    ui: &mut Ui,
    loc: &Localizer,
    f: &mut ScheduleForm,
    queues: &[QueueRecord],
) -> Outcome {
    let queue_name = |q: &QueueRecord| {
        if q.id == MAIN_QUEUE {
            loc.tr("ui-queue-main")
        } else if q.name.is_empty() {
            q.id.clone()
        } else {
            q.name.clone()
        }
    };
    egui::Grid::new("schedule-grid")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(loc.tr("ui-schedule-queue"));
            let current = queues
                .iter()
                .find(|q| q.id == f.queue)
                .map(queue_name)
                .unwrap_or_default();
            egui::ComboBox::from_id_salt("schedule-queue")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for q in queues {
                        ui.selectable_value(&mut f.queue, q.id.clone(), queue_name(q));
                    }
                });
            ui.end_row();
            text_row(
                ui,
                loc.tr("ui-schedule-start"),
                &mut f.start_at,
                Some("23:00".to_owned()),
            );
            text_row(
                ui,
                loc.tr("ui-schedule-stop"),
                &mut f.stop_at,
                Some("05:00".to_owned()),
            );
            text_row(
                ui,
                loc.tr("ui-schedule-days"),
                &mut f.days,
                Some(loc.tr("ui-schedule-days-hint")),
            );
            ui.label(loc.tr("ui-schedule-after"));
            egui::ComboBox::from_id_salt("schedule-after")
                .selected_text(loc.tr(&format!("after-done-{}", f.after_done)))
                .show_ui(ui, |ui| {
                    for a in AFTER_DONE {
                        ui.selectable_value(
                            &mut f.after_done,
                            a,
                            loc.tr(&format!("after-done-{a}")),
                        );
                    }
                });
            ui.end_row();
        });
    ui.label(RichText::new(loc.tr("ui-schedule-hint")).weak());
    let mut clear = false;
    let (ok, cancel) = buttons(ui, loc, "ui-btn-save", true);
    ui.horizontal(|ui| clear = ui.button(loc.tr("cmd-schedule-clear")).clicked());
    let mut out = Outcome {
        close: cancel,
        ..Default::default()
    };
    if clear {
        out.run
            .push(("schedule.clear", json!({ "queue": f.queue })));
        out.close = true;
    } else if ok {
        let mut args = serde_json::Map::new();
        args.insert("queue".into(), Value::from(f.queue.as_str()));
        args.insert("after_done".into(), Value::from(f.after_done));
        for (k, v) in [("start_at", &f.start_at), ("stop_at", &f.stop_at)] {
            if !v.trim().is_empty() {
                args.insert(k.into(), Value::from(v.trim()));
            }
        }
        let days: Vec<Value> = f
            .days
            .split(',')
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(Value::from)
            .collect();
        if !days.is_empty() {
            args.insert("days".into(), Value::Array(days));
        }
        out.run.push(("schedule.set", Value::Object(args)));
        out.close = true;
    }
    out
}

fn batch_dialog(ui: &mut Ui, loc: &Localizer, text: &mut String, start: &mut bool) -> Outcome {
    ui.label(loc.tr("ui-batch-hint"));
    ui.add(
        egui::TextEdit::multiline(text)
            .desired_rows(10)
            .desired_width(460.0)
            .code_editor(),
    );
    ui.checkbox(start, loc.tr("ui-add-start-now"));
    let (ok, cancel) = buttons(ui, loc, "ui-btn-add", !text.trim().is_empty());
    let mut out = Outcome {
        close: cancel,
        ..Default::default()
    };
    if ok {
        let t = text.trim();
        // A single line with `[a-b]` is a pattern; anything else is a list.
        if !t.contains('\n') && t.contains('[') {
            out.run
                .push(("batch.pattern", json!({ "pattern": t, "start": *start })));
        } else {
            out.run
                .push(("batch.import", json!({ "text": t, "start": *start })));
        }
        out.close = true;
    }
    out
}

fn grabber_dialog(ui: &mut Ui, loc: &Localizer, f: &mut GrabForm, jobs: &[GrabJob]) -> Outcome {
    let mut out = Outcome::default();
    egui::Grid::new("grab-grid")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            text_row(
                ui,
                loc.tr("ui-add-url"),
                &mut f.url,
                Some("https://".to_owned()),
            );
            ui.label(loc.tr("ui-grab-depth"));
            ui.add(egui::DragValue::new(&mut f.depth).range(0..=limits::MAX_GRABBER_DEPTH));
            ui.end_row();
            text_row(
                ui,
                loc.tr("ui-grab-extensions"),
                &mut f.extensions,
                Some("zip, pdf, iso".to_owned()),
            );
            ui.label("");
            ui.checkbox(&mut f.same_host, loc.tr("ui-grab-same-host"));
            ui.end_row();
        });
    let job = f.job.and_then(|id| jobs.iter().find(|j| j.id == id));
    if let Some(j) = job {
        ui.separator();
        let n = u64::try_from(j.files.len()).unwrap_or(u64::MAX);
        let key = if j.state == "running" {
            "ui-grab-running"
        } else {
            "ui-grab-found"
        };
        ui.label(loc.tr_args(
            key,
            &[("count", TrArg::Num(n)), ("pages", TrArg::Num(j.pages))],
        ));
        if let Some(e) = &j.error {
            ui.label(
                RichText::new(loc.message(&e.to_message())).color(ui.visuals().error_fg_color),
            );
        }
        egui::ScrollArea::vertical()
            .max_height(180.0)
            .show(ui, |ui| {
                for f in j.files.iter().take(MAX_LISTED_LINKS) {
                    ui.label(RichText::new(f).monospace().small());
                }
            });
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!f.url.trim().is_empty(), egui::Button::new(loc.tr("ui-grab-start")))
            .clicked()
        {
            let exts: Vec<Value> = f
                .extensions
                .split([',', ' '])
                .map(|e| e.trim().trim_start_matches('.'))
                .filter(|e| !e.is_empty())
                .map(Value::from)
                .collect();
            out.run.push((
                "grabber.start",
                json!({ "url": f.url.trim(), "depth": f.depth, "extensions": exts, "same_host": f.same_host }),
            ));
        }
        let ready = job.is_some_and(|j| j.state == "done" && !j.files.is_empty());
        if ui.add_enabled(ready, egui::Button::new(loc.tr("ui-grab-add"))).clicked()
            && let Some(id) = f.job
        {
            out.run.push(("grabber.add_results", json!({ "job": id, "start": true })));
            out.close = true;
        }
        if ui.button(loc.tr("ui-btn-close")).clicked() {
            out.close = true;
        }
    });
    out
}

fn remove_dialog(
    ui: &mut Ui,
    loc: &Localizer,
    id: &str,
    name: &str,
    delete_file: &mut bool,
) -> Outcome {
    ui.label(loc.tr_args("ui-remove-question", &[("name", TrArg::Str(name.into()))]));
    ui.checkbox(delete_file, loc.tr("ui-remove-delete-file"));
    let (ok, cancel) = buttons(ui, loc, "cmd-download-remove", true);
    let mut out = Outcome {
        close: cancel,
        ..Default::default()
    };
    if ok {
        out.run.push((
            "download.remove",
            json!({ "download_id": id, "delete_file": *delete_file, "confirm": true }),
        ));
        out.close = true;
    }
    out
}

fn properties_dialog(
    ui: &mut Ui,
    loc: &Localizer,
    f: &mut PropForm,
    downloads: &[DownloadView],
) -> Outcome {
    let mut out = Outcome::default();
    let Some(d) = downloads.iter().find(|d| d.id == f.id) else {
        out.close = true;
        return out;
    };
    let l = loc.locale();
    egui::Grid::new("prop-grid")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(loc.tr("ui-add-file-name"));
            ui.label(RichText::new(&d.file_name).strong());
            ui.end_row();
            ui.label(loc.tr("ui-detail-url"));
            ui.label(&d.url);
            ui.end_row();
            ui.label(loc.tr("ui-detail-folder"));
            ui.label(&d.dir);
            ui.end_row();
            ui.label(loc.tr("ui-col-size"));
            ui.label(
                d.size
                    .map_or_else(|| loc.tr("ui-unknown-size"), |s| format_bytes(l, s)),
            );
            ui.end_row();
            ui.label(loc.tr("ui-col-status"));
            ui.label(loc.tr(&d.status_key));
            ui.end_row();
            ui.label(loc.tr("ui-add-connections"));
            ui.add(
                egui::DragValue::new(&mut f.connections)
                    .range(1..=limits::MAX_CONNECTIONS_PER_DOWNLOAD),
            );
            ui.end_row();
            ui.label(loc.tr("ui-detail-limit"));
            ui.add(
                egui::DragValue::new(&mut f.limit_kib)
                    .speed(16)
                    .suffix(loc.tr("ui-unit-kib-s")),
            );
            ui.end_row();
            ui.label(loc.tr("ui-add-checksum"));
            ui.add(
                egui::TextEdit::singleline(&mut f.checksum)
                    .desired_width(380.0)
                    .hint_text("sha256:…"),
            );
            ui.end_row();
        });
    segment_bar(ui, d);
    if let Some(v) = &f.verify {
        ui.label(RichText::new(v).monospace().small());
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button(loc.tr("ui-btn-apply")).clicked() {
            if f.connections != d.connections {
                out.run.push((
                    "download.set_connections",
                    json!({ "download_id": d.id, "connections": f.connections }),
                ));
            }
            out.run.push((
                "limit.set_download",
                json!({ "download_id": d.id, "bytes_per_sec": f.limit_kib.saturating_mul(KIB) }),
            ));
        }
        let can_verify = d.status == "completed";
        if ui
            .add_enabled(can_verify, egui::Button::new(loc.tr("cmd-download-verify")))
            .clicked()
        {
            let mut args = json!({ "download_id": d.id });
            if !f.checksum.trim().is_empty()
                && let Some(a) = args.as_object_mut()
            {
                a.insert("checksum".into(), Value::from(f.checksum.trim()));
            }
            out.run.push(("download.verify", args));
        }
        if ui.button(loc.tr("ui-btn-close")).clicked() {
            out.close = true;
        }
    });
    out
}

fn captured_dialog(ui: &mut Ui, loc: &Localizer, links: &[String]) -> Outcome {
    let n = u64::try_from(links.len()).unwrap_or(u64::MAX);
    ui.label(loc.tr_args("ui-captured-question", &[("count", TrArg::Num(n))]));
    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(ui, |ui| {
            for l in links.iter().take(MAX_LISTED_LINKS) {
                ui.label(RichText::new(l).monospace().small());
            }
        });
    let (ok, cancel) = buttons(ui, loc, "ui-btn-add", true);
    let mut out = Outcome {
        close: cancel,
        ..Default::default()
    };
    if ok {
        out.run.push((
            "batch.import",
            json!({ "text": links.join("\n"), "start": true }),
        ));
        out.close = true;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kib_conversion_is_safe() {
        assert_eq!(kib_of(0), 0);
        assert_eq!(kib_of(2048), 2);
        assert_eq!(u64::MAX.saturating_mul(KIB), u64::MAX);
    }

    #[test]
    fn locale_tags_round_trip() {
        for l in [Locale::Id, Locale::En] {
            assert_eq!(Locale::parse(locale_tag(l)), Some(l));
        }
    }

    #[test]
    fn local_time_handles_garbage() {
        assert_eq!(local_time(i64::MAX), "");
        assert!(!local_time(0).is_empty());
    }
}

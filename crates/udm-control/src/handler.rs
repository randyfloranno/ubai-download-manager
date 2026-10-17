//! Request dispatch. Every method ends in the engine's command dispatcher or a
//! read-only engine view; UI methods go to an optional [`UiBridge`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Value, json};
use udm_core::{ArgKind, DownloadId, Message};
use udm_engine::{Engine, args_from_json, registry};

use crate::cmdline;
use crate::protocol::Request;

/// Boxed future, so handlers can be trait objects.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Something that answers control requests.
pub trait Handler: Send + Sync + 'static {
    /// Handles one request.
    fn handle(&self, req: Request) -> BoxFuture<'_, Result<Value, Message>>;
}

/// The GUI side of `ui.*` methods (`ui.set_locale`, `ui.screenshot`, `ui.open`).
pub trait UiBridge: Send + Sync + 'static {
    /// Handles a `ui.*` method.
    fn call(&self, method: String, params: Value) -> BoxFuture<'_, Result<Value, Message>>;
}

/// Full-access handler for the agent/CLI control port.
pub struct EngineHandler {
    engine: Engine,
    ui: Option<Arc<dyn UiBridge>>,
}

impl EngineHandler {
    /// Handler without a UI (CLI, headless).
    pub fn new(engine: Engine) -> Self {
        Self { engine, ui: None }
    }

    /// Handler that forwards `ui.*` to the GUI.
    pub fn with_ui(engine: Engine, ui: Arc<dyn UiBridge>) -> Self {
        Self {
            engine,
            ui: Some(ui),
        }
    }

    /// Runs one command with JSON args.
    pub async fn execute(&self, command: &str, args: &Value) -> Result<Value, Message> {
        let args = args_from_json(args)?;
        self.engine
            .execute(command, args)
            .await
            .map_err(|e| e.to_message())
    }

    async fn dispatch(&self, req: Request) -> Result<Value, Message> {
        match req.method.as_str() {
            "app.ping" => Ok(json!({ "pong": true, "version": env!("CARGO_PKG_VERSION") })),
            "engine.execute" => {
                let command = req
                    .str_param("command")
                    .ok_or_else(|| missing_param("command"))?;
                let args = req.params.get("args").cloned().unwrap_or(Value::Null);
                self.execute(command, &args).await
            }
            "engine.query" => self.query(&req),
            "engine.list_commands" => Ok(commands_json()),
            "cmdline.input" => {
                let text = req.str_param("text").ok_or_else(|| missing_param("text"))?;
                let (command, args) = cmdline::parse(text)?;
                let result = self
                    .engine
                    .execute(command, args)
                    .await
                    .map_err(|e| e.to_message())?;
                Ok(json!({ "command": command, "result": result }))
            }
            m if m.starts_with("ui.") => match &self.ui {
                Some(ui) => ui.call(req.method.clone(), req.params.clone()).await,
                None => Err(Message::new("error-control-no-ui")),
            },
            m => Err(Message::new("error-control-unknown-method").with_str("method", m)),
        }
    }

    fn query(&self, req: &Request) -> Result<Value, Message> {
        let what = req.str_param("what").ok_or_else(|| missing_param("what"))?;
        let v = match what {
            "downloads" => serde_json::to_value(self.engine.downloads()),
            "download" => {
                let id: DownloadId = req
                    .str_param("download_id")
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| missing_param("download_id"))?;
                let d = self.engine.download(id).ok_or_else(|| {
                    Message::new("error-engine-not-found").with_str("id", id.to_string())
                })?;
                serde_json::to_value(d)
            }
            "settings" => serde_json::to_value(self.engine.settings()),
            "queues" => serde_json::to_value(self.engine.queues()),
            "history" => serde_json::to_value(self.engine.history(req.str_param("search"))),
            "stats" => serde_json::to_value(self.engine.stats()),
            "grabber" => serde_json::to_value(self.engine.grab_jobs()),
            "commands" => Ok(commands_json()),
            other => {
                return Err(Message::new("error-control-bad-query").with_str("what", other));
            }
        };
        v.map_err(|_| Message::new("error-engine-internal"))
    }
}

impl Handler for EngineHandler {
    fn handle(&self, req: Request) -> BoxFuture<'_, Result<Value, Message>> {
        Box::pin(self.dispatch(req))
    }
}

fn missing_param(name: &str) -> Message {
    Message::new("error-control-missing-param").with_str("param", name)
}

fn kind_name(k: ArgKind) -> &'static str {
    match k {
        ArgKind::Str => "str",
        ArgKind::UInt => "uint",
        ArgKind::Bool => "bool",
        ArgKind::StrList => "list",
        ArgKind::DownloadId => "download_id",
        ArgKind::Enum(_) => "enum",
    }
}

/// The command registry as JSON.
pub fn commands_json() -> Value {
    let list: Vec<Value> = registry::COMMANDS
        .iter()
        .map(|c| {
            let args: Vec<Value> = c
                .args
                .iter()
                .map(|a| {
                    let options = match a.kind {
                        ArgKind::Enum(o) => json!(o),
                        _ => Value::Null,
                    };
                    json!({ "name": a.name, "kind": kind_name(a.kind), "required": a.required, "options": options })
                })
                .collect();
            json!({
                "id": c.id,
                "label_key": c.label_key,
                "description_key": c.description_key,
                "needs_confirmation": c.needs_confirmation,
                "args": args,
            })
        })
        .collect();
    Value::Array(list)
}

/// Restricted handler for the native messaging host: every request carries the
/// IPC token; only `app.ping` and the `capture.browser` command are allowed.
pub struct CaptureHandler {
    inner: EngineHandler,
    token: String,
}

impl CaptureHandler {
    /// New capture handler with the secret token from `ipc.json`.
    pub fn new(engine: Engine, token: String) -> Self {
        Self {
            inner: EngineHandler::new(engine),
            token,
        }
    }

    async fn dispatch(&self, req: Request) -> Result<Value, Message> {
        let given = req.str_param("token").unwrap_or("");
        if !constant_time_eq(given.as_bytes(), self.token.as_bytes()) {
            return Err(Message::new("error-control-unauthorized"));
        }
        match (req.method.as_str(), req.str_param("command")) {
            ("app.ping", _) => self.inner.dispatch(req).await,
            ("engine.execute", Some("capture.browser")) => {
                let args = req.params.get("args").cloned().unwrap_or(Value::Null);
                self.inner.execute("capture.browser", &args).await
            }
            _ => Err(Message::new("error-control-forbidden")),
        }
    }
}

impl Handler for CaptureHandler {
    fn handle(&self, req: Request) -> BoxFuture<'_, Result<Value, Message>> {
        Box::pin(self.dispatch(req))
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_compare() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn registry_as_json() {
        let v = commands_json();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), registry::COMMANDS.len());
        assert!(arr.iter().any(|c| c["id"] == "download.add"));
    }
}

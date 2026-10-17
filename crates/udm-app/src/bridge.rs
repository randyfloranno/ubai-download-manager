//! Control-port `ui.*` requests, handed to the GUI thread over a bounded channel.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use udm_control::{BoxFuture, UiBridge};
use udm_core::{Message, limits};

/// One request for the GUI; answered through `reply`.
pub struct UiRequest {
    /// Method (`ui.set_locale`, `ui.screenshot`, `ui.open`).
    pub method: String,
    /// Parameters.
    pub params: Value,
    /// Where the answer goes.
    pub reply: oneshot::Sender<Result<Value, Message>>,
}

/// The [`UiBridge`] given to the control server.
pub struct Bridge {
    tx: mpsc::Sender<UiRequest>,
    ctx: Arc<Mutex<Option<egui::Context>>>,
    timeout: Duration,
}

impl Bridge {
    /// New bridge and the receiver the GUI polls each frame.
    pub fn new() -> (Self, mpsc::Receiver<UiRequest>) {
        Self::with_timeout(Duration::from_secs(limits::UI_REPLY_TIMEOUT_SECS))
    }

    fn with_timeout(timeout: Duration) -> (Self, mpsc::Receiver<UiRequest>) {
        let (tx, rx) = mpsc::channel(limits::MAX_PENDING_UI_REQUESTS);
        let bridge = Self {
            tx,
            ctx: Arc::new(Mutex::new(None)),
            timeout,
        };
        (bridge, rx)
    }

    /// Lets the bridge wake the GUI when a request arrives.
    pub fn set_context(&self, ctx: egui::Context) {
        let mut slot = self
            .ctx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = Some(ctx);
    }

    /// Wakes the GUI so it renders a frame (and thus polls requests).
    pub fn wake(&self) {
        let slot = self
            .ctx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(ctx) = slot.as_ref() {
            ctx.request_repaint();
        }
    }
}

impl UiBridge for Bridge {
    fn call(&self, method: String, params: Value) -> BoxFuture<'_, Result<Value, Message>> {
        Box::pin(async move {
            let (reply, rx) = oneshot::channel();
            self.tx
                .try_send(UiRequest {
                    method,
                    params,
                    reply,
                })
                .map_err(|e| match e {
                    mpsc::error::TrySendError::Full(_) => Message::new("error-control-busy"),
                    mpsc::error::TrySendError::Closed(_) => Message::new("error-control-no-ui"),
                })?;
            self.wake();
            match tokio::time::timeout(self.timeout, rx).await {
                Ok(Ok(r)) => r,
                Ok(Err(_)) => Err(Message::new("error-control-no-ui")),
                Err(_) => Err(Message::new("error-ui-timeout")),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn answers_through_the_channel() {
        let (b, mut rx) = Bridge::new();
        let gui = tokio::spawn(async move {
            let r = rx.recv().await.unwrap();
            assert_eq!(r.method, "ui.open");
            r.reply.send(Ok(Value::from(1))).unwrap();
        });
        assert_eq!(
            b.call("ui.open".into(), Value::Null).await.unwrap(),
            Value::from(1)
        );
        gui.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn hostile_ui_bridge_gui_never_answers() {
        // `UI_REPLY_TIMEOUT_SECS`: a stuck GUI cannot hold control requests forever.
        let (b, _rx) = Bridge::new();
        let err = b
            .call("ui.screenshot".into(), Value::Null)
            .await
            .unwrap_err();
        assert_eq!(err.key, "error-ui-timeout");
        assert_eq!(
            b.timeout,
            Duration::from_secs(limits::UI_REPLY_TIMEOUT_SECS)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn hostile_ui_bridge_flood_is_bounded() {
        // `MAX_PENDING_UI_REQUESTS`: requests beyond the bound fail fast.
        let (b, _rx) = Bridge::with_timeout(Duration::from_secs(3600));
        let b = Arc::new(b);
        let mut waiting = Vec::new();
        for _ in 0..limits::MAX_PENDING_UI_REQUESTS {
            let b = Arc::clone(&b);
            waiting.push(tokio::spawn(async move {
                b.call("ui.open".into(), Value::Null).await
            }));
        }
        tokio::task::yield_now().await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        let err = b.call("ui.open".into(), Value::Null).await.unwrap_err();
        assert_eq!(err.key, "error-control-busy");
        for w in waiting {
            w.abort();
        }
    }

    #[tokio::test]
    async fn closed_gui_is_reported() {
        let (b, rx) = Bridge::new();
        drop(rx);
        let err = b.call("ui.open".into(), Value::Null).await.unwrap_err();
        assert_eq!(err.key, "error-control-no-ui");
    }
}

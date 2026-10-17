//! The HTTP(S) test server: listener, connection tracking, restart.

use crate::fixture::Fixture;
use crate::plan::HttpState;
use crate::{h2srv, http1, tls};
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::{AbortHandle, JoinHandle};
use tokio_rustls::TlsAcceptor;

/// TLS handshake timeout for incoming connections.
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Tracks spawned connection tasks so they can be aborted together.
#[derive(Default)]
pub(crate) struct Tracker {
    handles: Mutex<Vec<AbortHandle>>,
}

impl Tracker {
    pub fn track(&self, h: AbortHandle) {
        if let Ok(mut v) = self.handles.lock() {
            v.retain(|h| !h.is_finished());
            v.push(h);
        }
    }

    pub fn abort_all(&self) {
        if let Ok(mut v) = self.handles.lock() {
            for h in v.drain(..) {
                h.abort();
            }
        }
    }
}

/// Binds 127.0.0.1:`port`, retrying briefly (used when re-binding after restart).
pub(crate) async fn bind_loopback(port: u16) -> io::Result<TcpListener> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut last = None;
    for _ in 0..50 {
        match TcpListener::bind(addr).await {
            Ok(l) => return Ok(l),
            Err(e) => {
                if port == 0 {
                    return Err(e);
                }
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("bind failed")))
}

/// Local HTTP/1.1 (plain) or HTTPS (ALPN h2 + http/1.1) fault-injection server.
/// Binds 127.0.0.1 only. Dropping it stops all its tasks.
pub struct TestServer {
    state: Arc<HttpState>,
    tracker: Arc<Tracker>,
    acceptor: Option<TlsAcceptor>,
    cert_der: Option<Vec<u8>>,
    listener_task: Mutex<Option<JoinHandle<()>>>,
}

impl TestServer {
    /// Plain HTTP/1.1 on 127.0.0.1 with a free port.
    pub async fn start() -> io::Result<Self> {
        Self::bind(0, false).await
    }

    /// HTTPS on 127.0.0.1 with a free port; ALPN `h2` and `http/1.1`;
    /// self-signed certificate for `localhost` / `127.0.0.1` (see [`Self::cert_der`]).
    pub async fn start_tls() -> io::Result<Self> {
        Self::bind(0, true).await
    }

    /// Starts on a given port (`0` = pick a free one).
    pub async fn bind(port: u16, use_tls: bool) -> io::Result<Self> {
        let listener = bind_loopback(port).await?;
        let port = listener.local_addr()?.port();
        let (acceptor, cert_der) = if use_tls {
            let cfg = tls::server_config(&[b"h2", b"http/1.1"])?;
            (Some(TlsAcceptor::from(cfg)), Some(tls::test_cert_der()?))
        } else {
            (None, None)
        };
        let srv = TestServer {
            state: Arc::new(HttpState::new(port, use_tls)),
            tracker: Arc::new(Tracker::default()),
            acceptor,
            cert_der,
            listener_task: Mutex::new(None),
        };
        srv.spawn_accept(listener);
        Ok(srv)
    }

    fn spawn_accept(&self, listener: TcpListener) {
        let state = self.state.clone();
        let tracker = self.tracker.clone();
        let acceptor = self.acceptor.clone();
        let task = tokio::spawn(async move {
            loop {
                let tcp = match listener.accept().await {
                    Ok((tcp, _)) => tcp,
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        continue;
                    }
                };
                let _ = tcp.set_nodelay(true);
                let state = state.clone();
                let tr = tracker.clone();
                let acceptor = acceptor.clone();
                let h = tokio::spawn(async move {
                    match acceptor {
                        None => http1::serve(state, tcp).await,
                        Some(acc) => {
                            let Ok(Ok(tls)) =
                                tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, acc.accept(tcp)).await
                            else {
                                return;
                            };
                            let is_h2 = tls.get_ref().1.alpn_protocol() == Some(b"h2".as_slice());
                            if is_h2 {
                                h2srv::serve(state, tr, tls).await;
                            } else {
                                http1::serve(state, tls).await;
                            }
                        }
                    }
                });
                tracker.track(h.abort_handle());
            }
        });
        if let Some(old) = self
            .listener_task
            .lock()
            .ok()
            .and_then(|mut t| t.replace(task))
        {
            old.abort();
        }
    }

    /// The bound port.
    pub fn port(&self) -> u16 {
        self.state.port
    }

    /// `http://127.0.0.1:<port>` or `https://127.0.0.1:<port>` (no trailing slash).
    pub fn base_url(&self) -> String {
        let scheme = if self.state.tls { "https" } else { "http" };
        format!("{scheme}://127.0.0.1:{}", self.state.port)
    }

    /// `base_url() + path` (`path` should start with `/`).
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url(), path)
    }

    /// DER of the self-signed certificate (TLS servers only).
    pub fn cert_der(&self) -> Option<Vec<u8>> {
        self.cert_der.clone()
    }

    /// Whether this is the TLS server.
    pub fn is_tls(&self) -> bool {
        self.state.tls
    }

    /// Changes the content version of fixture `name` (content + ETag + Last-Modified).
    pub fn set_version(&self, name: &str, v: u32) {
        if let Ok(mut m) = self.state.versions.lock() {
            m.insert(name.to_string(), v);
        }
    }

    /// Current content version of fixture `name`.
    pub fn version(&self, name: &str) -> u32 {
        self.state.version(name)
    }

    /// The fixture `name` as this server currently serves it.
    pub fn fixture(&self, name: &str) -> Option<Fixture> {
        self.state.fixture(name)
    }

    /// Base URL (e.g. a plain server's `base_url()`) that `/downgrade` redirects to.
    pub fn set_downgrade_target(&self, base_url: &str) {
        if let Ok(mut t) = self.state.downgrade_target.lock() {
            *t = Some(base_url.to_string());
        }
    }

    /// Forgets fault counters (`drop_times`) so faults fire again.
    pub fn reset_faults(&self) {
        if let Ok(mut h) = self.state.fault_hits.lock() {
            h.clear();
        }
    }

    /// Aborts all active connections and the listener, re-binds the same port
    /// and continues serving (state such as versions is kept).
    ///
    /// # Panics
    /// When the port cannot be re-bound (test-only crate); see [`Self::try_restart`].
    #[allow(clippy::expect_used)]
    pub async fn restart(&self) {
        self.try_restart()
            .await
            .expect("udm-testsrv: re-bind after restart failed");
    }

    /// Like [`Self::restart`] but returns the re-bind error.
    pub async fn try_restart(&self) -> io::Result<()> {
        let old = self.listener_task.lock().ok().and_then(|mut t| t.take());
        if let Some(old) = old {
            old.abort();
            let _ = old.await;
        }
        self.tracker.abort_all();
        tokio::task::yield_now().await;
        let listener = bind_loopback(self.state.port).await?;
        self.spawn_accept(listener);
        Ok(())
    }

    /// Number of HTTP requests received (all routes, both protocols).
    pub fn request_count(&self) -> usize {
        self.state.requests.load(Ordering::Relaxed)
    }

    /// Stops the listener and all connections.
    pub fn shutdown(self) {
        drop(self);
    }

    fn stop(&self) {
        if let Some(h) = self.listener_task.lock().ok().and_then(|mut t| t.take()) {
            h.abort();
        }
        self.tracker.abort_all();
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop();
    }
}

//! HTTP/2 transport (hyper server) executing the same [`Plan`]s.

use crate::plan::{BodyGen, HttpState, Next, Plan, route};
use crate::server::Tracker;
use crate::util::Req;
use bytes::Bytes;
use http_body_util::{BodyExt, Empty, StreamBody, combinators::UnsyncBoxBody};
use hyper::body::{Frame, Incoming};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::io::{AsyncRead, AsyncWrite};

type RespBody = UnsyncBoxBody<Bytes, io::Error>;

/// Executor that tracks spawned stream tasks so `restart`/`shutdown` abort them.
#[derive(Clone)]
struct TrackedExec(Arc<Tracker>);

impl<F> hyper::rt::Executor<F> for TrackedExec
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn execute(&self, fut: F) {
        let h = tokio::spawn(fut);
        self.0.track(h.abort_handle());
    }
}

pub(crate) async fn serve<S>(state: Arc<HttpState>, tracker: Arc<Tracker>, io: S)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let svc = hyper::service::service_fn(move |req: hyper::Request<Incoming>| {
        let state = state.clone();
        async move { Ok::<_, Infallible>(handle(state, req).await) }
    });
    let _ = hyper::server::conn::http2::Builder::new(TrackedExec(tracker))
        .max_send_buf_size(64 * 1024)
        .serve_connection(TokioIo::new(io), svc)
        .await;
}

async fn handle(state: Arc<HttpState>, req: hyper::Request<Incoming>) -> hyper::Response<RespBody> {
    state.requests.fetch_add(1, Ordering::Relaxed);
    let target = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let headers = req
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_ascii_lowercase(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    let r = Req::new(req.method().as_str(), &target, headers);
    let resp = match route(&state, &r) {
        Plan::Stall => std::future::pending::<crate::plan::Resp>().await,
        Plan::Respond(resp) => resp,
    };
    let mut b = hyper::Response::builder().status(resp.status);
    for (k, v) in &resp.headers {
        if k.eq_ignore_ascii_case("connection") {
            continue;
        }
        if let (Ok(name), Ok(val)) = (
            http::HeaderName::from_bytes(k.as_bytes()),
            http::HeaderValue::from_bytes(v),
        ) {
            b = b.header(name, val);
        }
    }
    if resp.endless_headers {
        // HTTP/2 cannot stream a header block forever; send a very long list.
        for i in 0..4096 {
            b = b.header(format!("x-filler-{i}"), "f".repeat(48));
        }
    }
    let body: RespBody = if resp.head_only {
        Empty::<Bytes>::new()
            .map_err(|never| match never {})
            .boxed_unsync()
    } else {
        let body_gen = BodyGen::new(resp.body, resp.cut, resp.slow);
        let stream =
            futures_util::stream::unfold(Some(body_gen), |g: Option<BodyGen>| async move {
                let mut g = g?;
                match g.next().await {
                    Next::Data(d) => Some((Ok(Frame::data(d)), Some(g))),
                    Next::Done => None,
                    Next::Cut => {
                        // Let headers and the first N bytes reach the peer before the reset.
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        Some((Err(io::Error::other("udm-testsrv: rst_after")), None))
                    }
                }
            });
        StreamBody::new(stream).boxed_unsync()
    };
    match b.body(body) {
        Ok(r) => r,
        Err(_) => {
            let mut r = hyper::Response::new(
                Empty::<Bytes>::new()
                    .map_err(|never| match never {})
                    .boxed_unsync(),
            );
            *r.status_mut() = http::StatusCode::INTERNAL_SERVER_ERROR;
            r
        }
    }
}

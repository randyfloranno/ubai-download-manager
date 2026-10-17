//! HTTP/1.1 and HTTP/2 requests over hyper's connection API, with redirect
//! policy, authentication retry and header bounds.

use std::collections::HashSet;

use bytes::Bytes;
use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use http::{Method, StatusCode};
use http_body_util::Empty;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use udm_core::{Secret, limits};
use udm_proto::{Challenge, is_downgrade, resolve_redirect, split_credentials};

use crate::client::Client;
use crate::config::{Protocol, Request};
use crate::connect::{self, Target, with_timeout};
use crate::error::{NetError, TimeoutPhase, bounded_text};

type ReqBody = Empty<Bytes>;

/// Keeps the connection handle alive while a body is read.
pub(crate) enum KeepAlive {
    /// HTTP/1.1 sender (the connection task ends when the body is done).
    H1(#[allow(dead_code)] hyper::client::conn::http1::SendRequest<ReqBody>),
    /// HTTP/2 sender.
    H2(#[allow(dead_code)] hyper::client::conn::http2::SendRequest<ReqBody>),
}

/// A response head with its body and context.
pub(crate) struct HttpResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Incoming,
    pub(crate) keep: KeepAlive,
    pub(crate) url: url::Url,
    pub(crate) protocol: Protocol,
}

impl HttpResponse {
    /// First value of a header as text, if valid ASCII.
    pub(crate) fn header(&self, name: HeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// Fails when the body uses a non-identity `Content-Encoding`.
    pub(crate) fn check_identity_encoding(&self) -> Result<(), NetError> {
        for v in self.headers.get_all(header::CONTENT_ENCODING) {
            let s = v.to_str().unwrap_or("?");
            for enc in s.split(',') {
                let enc = enc.trim();
                if !enc.is_empty() && !enc.eq_ignore_ascii_case("identity") {
                    return Err(NetError::ContentEncoding {
                        encoding: bounded_text(enc, 32),
                    });
                }
            }
        }
        Ok(())
    }

    /// Parsed `Content-Length`.
    pub(crate) fn content_length(&self) -> Option<u64> {
        self.header(header::CONTENT_LENGTH)
            .and_then(udm_proto::parse_content_length)
    }
}

/// Result of following an HTTP request: a response, or a redirect to FTP.
pub(crate) enum Outcome {
    Http(HttpResponse),
    Ftp(url::Url),
}

fn map_hyper_error(e: &hyper::Error) -> NetError {
    // `is_parse_too_large` needs hyper's server feature; the message is stable.
    if e.is_parse() && e.to_string().contains("too large") {
        NetError::HeaderTooLarge {
            max: u64::try_from(limits::MAX_HEADER_BYTES).unwrap_or(u64::MAX),
        }
    } else if e.is_parse() || e.is_parse_status() {
        NetError::Protocol
    } else if e.is_timeout() {
        NetError::Timeout {
            phase: TimeoutPhase::FirstByte,
            ms: 0,
        }
    } else if e.is_incomplete_message() || e.is_closed() || e.is_canceled() {
        NetError::Io
    } else {
        // HTTP/2 stream reset (RST_STREAM / GOAWAY) and other transport failures.
        NetError::Stream
    }
}

fn header_value(s: &str) -> Result<HeaderValue, NetError> {
    if !udm_proto::header_value_ok(s) {
        return Err(NetError::InvalidRequest);
    }
    HeaderValue::from_str(s).map_err(|_| NetError::InvalidRequest)
}

fn host_header(u: &url::Url) -> Result<String, NetError> {
    let host = u.host_str().ok_or(NetError::InvalidRequest)?;
    Ok(match u.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_owned(),
    })
}

/// Path and query of a URL (origin-form request target).
pub(crate) fn request_target(u: &url::Url) -> String {
    match u.query() {
        Some(q) => format!("{}?{q}", u.path()),
        None => u.path().to_owned(),
    }
}

/// Checks the received head against the header limits (HTTP/2 has no per-count knob).
fn check_header_bounds(h: &HeaderMap) -> Result<(), NetError> {
    if h.len() > limits::MAX_HEADER_COUNT {
        return Err(NetError::TooManyHeaders {
            max: u64::try_from(limits::MAX_HEADER_COUNT).unwrap_or(u64::MAX),
        });
    }
    let bytes = h.iter().fold(0usize, |acc, (k, v)| {
        acc.saturating_add(k.as_str().len())
            .saturating_add(v.len())
            .saturating_add(4)
    });
    if bytes > limits::MAX_HEADER_BYTES {
        return Err(NetError::HeaderTooLarge {
            max: u64::try_from(limits::MAX_HEADER_BYTES).unwrap_or(u64::MAX),
        });
    }
    Ok(())
}

fn random_hex(bytes: usize) -> Result<String, NetError> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|_| NetError::Io)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

fn is_redirect(s: StatusCode) -> bool {
    matches!(s.as_u16(), 301 | 302 | 303 | 307 | 308)
}

/// Builds the `Authorization` value answering the best supported challenge.
fn answer_challenge(
    resp: &HttpResponse,
    user: &str,
    pass: &str,
    url: &url::Url,
) -> Result<String, NetError> {
    let mut challenges: Vec<Challenge> = Vec::new();
    for v in resp
        .headers
        .get_all(header::WWW_AUTHENTICATE)
        .iter()
        .take(limits::MAX_HEADER_COUNT)
    {
        if let Ok(s) = v.to_str()
            && let Ok(mut c) = udm_proto::parse_www_authenticate(s)
        {
            challenges.append(&mut c);
        }
    }
    let digest = challenges.iter().find_map(|c| match c {
        Challenge::Digest(d) => Some(d),
        Challenge::Basic { .. } => None,
    });
    if let Some(d) = digest {
        let cnonce = random_hex(16)?;
        return Ok(udm_proto::digest_authorization(
            d,
            user,
            pass,
            "GET",
            &request_target(url),
            &cnonce,
            1,
        ));
    }
    if challenges
        .iter()
        .any(|c| matches!(c, Challenge::Basic { .. }))
    {
        return Ok(udm_proto::basic_authorization(user, pass));
    }
    Err(NetError::AuthFailed)
}

impl Client {
    /// Sends one GET on a fresh connection and returns the response head.
    async fn send_once(
        &self,
        url: &url::Url,
        extra: &[(HeaderName, HeaderValue)],
    ) -> Result<HttpResponse, NetError> {
        let https = match url.scheme() {
            "http" => false,
            "https" => true,
            s => {
                return Err(NetError::UnsupportedScheme {
                    scheme: bounded_text(s, 16),
                });
            }
        };
        let target = Target::from_url(url, if https { 443 } else { 80 })?;
        let t = &self.cfg.timeouts;
        let (io, _) = connect::open(self.cfg.proxy.as_ref(), t, &target).await?;
        let (io, h2) = if https {
            connect::tls(&self.tls.http, t, io, &target.host).await?
        } else {
            (io, false)
        };

        let mut clean = url.clone();
        clean.set_fragment(None);
        let mut builder = http::Request::builder().method(Method::GET);
        if h2 {
            builder = builder.uri(clean.as_str());
        } else {
            builder = builder
                .uri(request_target(&clean))
                .header(header::HOST, header_value(&host_header(&clean)?)?);
        }
        builder = builder
            .header(header::USER_AGENT, header_value(&self.cfg.user_agent)?)
            .header(header::ACCEPT, HeaderValue::from_static("*/*"))
            .header(
                header::ACCEPT_ENCODING,
                HeaderValue::from_static("identity"),
            );
        for (k, v) in extra {
            builder = builder.header(k, v);
        }
        let request = builder
            .body(Empty::<Bytes>::new())
            .map_err(|_| NetError::InvalidRequest)?;

        let io = TokioIo::new(io);
        let first_byte = t.first_byte();
        let (resp, keep, protocol) = if h2 {
            let mut b = hyper::client::conn::http2::Builder::new(TokioExecutor::new());
            b.max_header_list_size(u32::try_from(limits::MAX_HEADER_BYTES).unwrap_or(u32::MAX));
            let (mut sender, conn) = with_timeout(TimeoutPhase::FirstByte, first_byte, async {
                b.handshake(io).await.map_err(|e| map_hyper_error(&e))
            })
            .await?;
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    tracing::debug!(error = %e, "http2 connection ended with error");
                }
            });
            let resp = with_timeout(TimeoutPhase::FirstByte, first_byte, async {
                sender
                    .send_request(request)
                    .await
                    .map_err(|e| map_hyper_error(&e))
            })
            .await?;
            (resp, KeepAlive::H2(sender), Protocol::Http2)
        } else {
            let mut b = hyper::client::conn::http1::Builder::new();
            b.max_headers(limits::MAX_HEADER_COUNT)
                .max_header_size(limits::MAX_HEADER_BYTES);
            let (mut sender, conn) = with_timeout(TimeoutPhase::FirstByte, first_byte, async {
                b.handshake(io).await.map_err(|e| map_hyper_error(&e))
            })
            .await?;
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    tracing::debug!(error = %e, "http1 connection ended with error");
                }
            });
            let resp = with_timeout(TimeoutPhase::FirstByte, first_byte, async {
                sender
                    .send_request(request)
                    .await
                    .map_err(|e| map_hyper_error(&e))
            })
            .await?;
            (resp, KeepAlive::H1(sender), Protocol::Http1)
        };
        let (parts, body) = resp.into_parts();
        check_header_bounds(&parts.headers)?;
        Ok(HttpResponse {
            status: parts.status,
            headers: parts.headers,
            body,
            keep,
            url: url.clone(),
            protocol,
        })
    }

    /// GET with redirects (bounded, loop-checked, downgrade policy) and one
    /// authentication retry. `range` is the `Range` header value, if any.
    pub(crate) async fn http_get(
        &self,
        req: &Request,
        range: Option<&str>,
    ) -> Result<Outcome, NetError> {
        let (mut url, url_creds) = split_credentials(&req.url);
        let url_creds: Option<(String, Secret)> = url_creds.map(|(u, p)| (u, Secret::new(p)));
        let creds = req.credentials.as_ref().or(url_creds.as_ref());
        let origin = url.origin();
        let allow_downgrade = req.allow_downgrade || self.cfg.allow_downgrade;

        let mut base: Vec<(HeaderName, HeaderValue)> = Vec::new();
        if let Some(r) = range {
            base.push((header::RANGE, header_value(r)?));
        }
        if let Some(r) = &req.referer {
            base.push((header::REFERER, header_value(r)?));
        }
        let cookie = match &req.cookie {
            Some(c) if c.expose().len() > limits::MAX_COOKIE_BYTES => {
                return Err(NetError::InvalidRequest);
            }
            Some(c) => {
                let mut v = header_value(c.expose())?;
                v.set_sensitive(true);
                Some(v)
            }
            None => None,
        };

        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(url.as_str().to_owned());
        let mut redirects: u32 = 0;
        loop {
            if matches!(url.scheme(), "ftp" | "ftps") {
                return Ok(Outcome::Ftp(url));
            }
            // Cookies and credentials only go to the origin the user asked for.
            let same_origin = url.origin() == origin;
            let mut headers = base.clone();
            if same_origin && let Some(c) = &cookie {
                headers.push((header::COOKIE, c.clone()));
            }
            let mut auth_tried = false;
            let resp = loop {
                let resp = self.send_once(&url, &headers).await?;
                if resp.status != StatusCode::UNAUTHORIZED {
                    break resp;
                }
                if auth_tried {
                    return Err(NetError::AuthFailed);
                }
                let Some((user, pass)) = creds.filter(|_| same_origin) else {
                    return Err(NetError::AuthRequired);
                };
                let value = answer_challenge(&resp, user, pass.expose(), &url)?;
                let mut v = header_value(&value)?;
                v.set_sensitive(true);
                headers.push((header::AUTHORIZATION, v));
                auth_tried = true;
            };
            if resp.status == StatusCode::PROXY_AUTHENTICATION_REQUIRED {
                return Err(NetError::ProxyAuth);
            }
            if !is_redirect(resp.status) {
                return Ok(Outcome::Http(resp));
            }
            let Some(location) = resp.header(header::LOCATION) else {
                return Err(NetError::RedirectInvalid);
            };
            redirects = redirects.saturating_add(1);
            if redirects > limits::MAX_REDIRECTS {
                return Err(NetError::TooManyRedirects {
                    max: limits::MAX_REDIRECTS,
                });
            }
            let next = resolve_redirect(&url, location).map_err(|_| NetError::RedirectInvalid)?;
            let (mut next, _) = split_credentials(&next);
            next.set_fragment(None);
            if is_downgrade(&url, &next) && !allow_downgrade {
                return Err(NetError::DowngradeRefused);
            }
            if !visited.insert(next.as_str().to_owned()) {
                return Err(NetError::RedirectLoop);
            }
            tracing::debug!(from = %url, to = %next, "following redirect");
            url = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_keeps_query_drops_fragment() {
        let u = url::Url::parse("http://h/a/b.bin?x=1#frag").unwrap();
        assert_eq!(request_target(&u), "/a/b.bin?x=1");
    }

    #[test]
    fn hostile_header_value_with_crlf_rejected() {
        assert!(header_value("a\r\nX-Evil: 1").is_err());
    }

    #[test]
    fn hostile_too_many_headers_rejected() {
        let mut h = HeaderMap::new();
        for i in 0..=limits::MAX_HEADER_COUNT {
            h.insert(
                HeaderName::from_bytes(format!("x-h{i}").as_bytes()).unwrap(),
                HeaderValue::from_static("v"),
            );
        }
        assert!(matches!(
            check_header_bounds(&h),
            Err(NetError::TooManyHeaders { .. })
        ));
    }
}

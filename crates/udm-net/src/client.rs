//! The public [`Client`]: probe, ranged fetch and grabber page fetch.

use http::StatusCode;
use http::header;
use http_body_util::BodyExt;
use udm_core::{ByteRange, limits};
use udm_proto::{ContentRange, EntityTag};

use crate::body::BodyStream;
use crate::config::{NetConfig, ProbeInfo, Request};
use crate::error::{NetError, TimeoutPhase, bounded_text};
use crate::http::{HttpResponse, Outcome};
use crate::tls::{self, TlsConfigs};

/// Response body of [`Client::fetch`].
#[derive(Debug)]
pub enum FetchBody {
    /// `206` with a verified `Content-Range` (or an FTP `REST` read): exactly the requested range.
    Partial(BodyStream),
    /// The whole file from offset 0: no range was asked, or the server ignored it (`200`).
    Full(BodyStream),
}

/// Network client. Each request uses its own connection.
pub struct Client {
    pub(crate) cfg: NetConfig,
    pub(crate) tls: TlsConfigs,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("cfg", &self.cfg).finish()
    }
}

fn is_ftp(u: &url::Url) -> bool {
    matches!(u.scheme(), "ftp" | "ftps")
}

fn with_url(req: &Request, url: url::Url) -> Request {
    let mut r = req.clone();
    r.url = url;
    r
}

impl Client {
    /// Builds a client: TLS verifier (platform roots + `extra_root_der` + pinned exceptions).
    pub fn new(cfg: NetConfig) -> Result<Self, NetError> {
        let tls = tls::build(&cfg.extra_root_der, &cfg.tls_exceptions)?;
        Ok(Self { cfg, tls })
    }

    /// Learns size, range support, validators and file name. HTTP: `GET` with
    /// `Range: bytes=0-`, headers only, following redirects. FTP: `SIZE`/`MDTM`/`REST 0`.
    pub async fn probe(&self, req: &Request) -> Result<ProbeInfo, NetError> {
        if is_ftp(&req.url) {
            return self.ftp_probe(req).await;
        }
        let resp = match self.http_get(req, Some("bytes=0-")).await? {
            Outcome::Http(r) => r,
            Outcome::Ftp(u) => return self.ftp_probe(&with_url(req, u)).await,
        };
        resp.check_identity_encoding()?;
        let (size, accepts_ranges) = match resp.status {
            StatusCode::PARTIAL_CONTENT => {
                match resp.header(header::CONTENT_RANGE).map(ContentRange::parse) {
                    Some(Ok(cr)) => (cr.total, cr.start == 0 && cr.total.is_some()),
                    _ => (None, false),
                }
            }
            StatusCode::OK => (resp.content_length(), false),
            StatusCode::RANGE_NOT_SATISFIABLE => {
                // `bytes */0`: an empty file.
                let total = resp
                    .header(header::CONTENT_RANGE)
                    .and_then(|v| v.trim().strip_prefix("bytes */"))
                    .and_then(udm_proto::parse_content_length);
                match total {
                    Some(0) => (Some(0), false),
                    _ => {
                        return Err(NetError::HttpStatus {
                            status: resp.status.as_u16(),
                        });
                    }
                }
            }
            s => return Err(NetError::HttpStatus { status: s.as_u16() }),
        };
        let etag = resp.header(header::ETAG).and_then(EntityTag::parse);
        let last_modified = resp
            .header(header::LAST_MODIFIED)
            .filter(|v| udm_proto::header_value_ok(v))
            .map(str::to_owned);
        let filename = resp
            .header(header::CONTENT_DISPOSITION)
            .and_then(udm_proto::filename_from_content_disposition);
        let content_type = resp
            .header(header::CONTENT_TYPE)
            .filter(|v| udm_proto::header_value_ok(v))
            .map(str::to_owned);
        let HttpResponse { url, protocol, .. } = resp;
        Ok(ProbeInfo {
            final_url: url,
            size,
            accepts_ranges,
            etag,
            last_modified,
            filename,
            content_type,
            protocol,
        })
    }

    /// Fetches file data. With `range`, a verified `206` yields [`FetchBody::Partial`];
    /// a `200` yields [`FetchBody::Full`] (the engine restarts from offset 0).
    /// A strong `validator` that differs from the response ETag is
    /// [`NetError::RemoteChanged`]. `Content-Range` must match `range` and `expected_total`.
    pub async fn fetch(
        &self,
        req: &Request,
        range: Option<ByteRange>,
        expected_total: Option<u64>,
        validator: Option<&EntityTag>,
    ) -> Result<FetchBody, NetError> {
        if range.is_some_and(ByteRange::is_empty) {
            return Err(NetError::InvalidRequest);
        }
        if is_ftp(&req.url) {
            return self.fetch_ftp(req, range, expected_total).await;
        }
        let range_value = range.and_then(udm_proto::range_header);
        let resp = match self.http_get(req, range_value.as_deref()).await? {
            Outcome::Http(r) => r,
            Outcome::Ftp(u) => {
                return self
                    .fetch_ftp(&with_url(req, u), range, expected_total)
                    .await;
            }
        };
        if !matches!(resp.status, StatusCode::OK | StatusCode::PARTIAL_CONTENT) {
            return Err(NetError::HttpStatus {
                status: resp.status.as_u16(),
            });
        }
        resp.check_identity_encoding()?;
        if let (Some(v), Some(got)) = (
            validator.filter(|v| !v.weak),
            resp.header(header::ETAG).and_then(EntityTag::parse),
        ) && !got.weak
            && !v.strong_eq(&got)
        {
            return Err(NetError::RemoteChanged);
        }
        let idle = self.cfg.timeouts.idle();
        let content_length = resp.content_length();
        match (resp.status, range) {
            (StatusCode::PARTIAL_CONTENT, Some(r)) => {
                let cr = resp.header(header::CONTENT_RANGE);
                match expected_total {
                    Some(total) => udm_proto::verify_content_range(r, total, cr)
                        .map_err(|detail| NetError::BadContentRange { detail })?,
                    None => {
                        let parsed = ContentRange::parse(cr.ok_or(NetError::BadContentRange {
                            detail: udm_proto::ProtoError::ContentRangeMissing,
                        })?)
                        .map_err(|detail| NetError::BadContentRange { detail })?;
                        if parsed.byte_range() != Some(r) {
                            return Err(NetError::BadContentRange {
                                detail: udm_proto::ProtoError::ContentRangeInvalid,
                            });
                        }
                    }
                }
                // A Content-Length that contradicts Content-Range: the framing
                // cannot be trusted.
                if content_length.is_some_and(|cl| cl != r.len()) {
                    return Err(NetError::BadContentRange {
                        detail: udm_proto::ProtoError::ContentRangeInvalid,
                    });
                }
                let HttpResponse { body, keep, .. } = resp;
                Ok(FetchBody::Partial(BodyStream::http(
                    body,
                    keep,
                    Some(r.len()),
                    idle,
                )))
            }
            (StatusCode::PARTIAL_CONTENT, None) => Err(NetError::Protocol),
            _ => {
                let HttpResponse { body, keep, .. } = resp;
                let expected = expected_total.or(content_length);
                Ok(FetchBody::Full(BodyStream::http(
                    body, keep, expected, idle,
                )))
            }
        }
    }

    async fn fetch_ftp(
        &self,
        req: &Request,
        range: Option<ByteRange>,
        expected_total: Option<u64>,
    ) -> Result<FetchBody, NetError> {
        let (body, partial) = self.ftp_fetch(req, range, expected_total).await?;
        Ok(if partial {
            FetchBody::Partial(body)
        } else {
            FetchBody::Full(body)
        })
    }

    /// Fetches an HTML page for the site grabber, following redirects. The body is
    /// truncated at `max_bytes` (and never more than `limits::MAX_GRABBER_PAGE_BYTES`).
    /// Returns the final URL and the page as lossy UTF-8.
    pub async fn fetch_page(
        &self,
        req: &Request,
        max_bytes: usize,
    ) -> Result<(url::Url, String), NetError> {
        if is_ftp(&req.url) {
            return Err(NetError::UnsupportedScheme {
                scheme: bounded_text(req.url.scheme(), 16),
            });
        }
        let resp = match self.http_get(req, None).await? {
            Outcome::Http(r) => r,
            Outcome::Ftp(u) => {
                return Err(NetError::UnsupportedScheme {
                    scheme: bounded_text(u.scheme(), 16),
                });
            }
        };
        if resp.status != StatusCode::OK {
            return Err(NetError::HttpStatus {
                status: resp.status.as_u16(),
            });
        }
        resp.check_identity_encoding()?;
        if let Some(ct) = resp.header(header::CONTENT_TYPE) {
            let mime = ct
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if !mime.is_empty() && mime != "text/html" && mime != "application/xhtml+xml" {
                return Err(NetError::NotHtml {
                    content_type: bounded_text(&mime, 64),
                });
            }
        }
        let cap = max_bytes.min(limits::MAX_GRABBER_PAGE_BYTES);
        let HttpResponse { mut body, url, .. } = resp;
        let idle = self.cfg.timeouts.idle();
        let mut page: Vec<u8> = Vec::new();
        while page.len() < cap {
            let frame = match tokio::time::timeout(idle, body.frame()).await {
                Err(_) => {
                    return Err(NetError::Timeout {
                        phase: TimeoutPhase::Idle,
                        ms: self.cfg.timeouts.idle_ms,
                    });
                }
                Ok(None) => break,
                Ok(Some(Err(_))) => return Err(NetError::Stream),
                Ok(Some(Ok(f))) => f,
            };
            if let Ok(data) = frame.into_data() {
                let room = cap.saturating_sub(page.len());
                let take = data.len().min(room);
                page.extend_from_slice(data.get(..take).unwrap_or(&[]));
            }
        }
        Ok((url, String::from_utf8_lossy(&page).into_owned()))
    }
}

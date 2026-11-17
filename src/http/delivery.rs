//! HTTP file semantics shared by public artifact downloads. Nginx uses the same
//! mtime/length validators as the direct adapter (immutable files only).
use super::{AppState, error::ApiError};
use crate::{config::DeliveryMode, services::distribution::PublicDownload};
use axum::{
    body::Body,
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures_util::{StreamExt, stream};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

pub(super) fn disposition(filename: &str) -> String {
    let fallback: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded: String = filename
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'&'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
            {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}
fn matches_tag(headers: &HeaderMap, name: header::HeaderName, etag: &str, weak: bool) -> bool {
    headers
        .get_all(name)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|s| s.split(','))
        .map(str::trim)
        .any(|t| {
            t == "*"
                || if weak {
                    t.strip_prefix("W/").unwrap_or(t) == etag
                } else {
                    t == etag
                }
        })
}
fn date(headers: &HeaderMap, name: header::HeaderName) -> Option<SystemTime> {
    headers
        .get(name)?
        .to_str()
        .ok()
        .and_then(|s| httpdate::parse_http_date(s).ok())
}
/// Multiple ranges are ignored with 200 (RFC 9110 §14); never emit multipart.
/// Malformed/unsatisfiable single byte ranges receive 416.
fn range(value: &str, length: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(value) = value.strip_prefix("bytes=") else {
        return Ok(None);
    };
    if value.contains(',') {
        return Ok(None);
    }
    let (start, end) = value.split_once('-').ok_or(())?;
    let number = |s: &str| -> Result<u64, ()> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(());
        }
        s.parse().map_err(|_| ())
    };
    if start.is_empty() {
        let suffix = number(end)?;
        if suffix == 0 || length == 0 {
            return Err(());
        }
        return Ok(Some((length.saturating_sub(suffix), length - 1)));
    }
    let start = number(start)?;
    let end = if end.is_empty() {
        length.saturating_sub(1)
    } else {
        number(end)?
    };
    if start >= length || end < start {
        return Err(());
    }
    Ok(Some((start, end.min(length - 1))))
}
pub async fn deliver(
    state: &AppState,
    download: PublicDownload,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let permit = state.distribution.permit()?;
    let mut file = state.distribution.open(&download).await?;
    let metadata = file
        .metadata()
        .await
        .map_err(|_| ApiError::StorageUnavailable)?;
    let modified = metadata
        .modified()
        .map_err(|_| ApiError::StorageUnavailable)?;
    let seconds = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::StorageUnavailable)?
        .as_secs();
    let modified = UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    let length = download.artifact.size_bytes as u64;
    let etag = format!("\"{seconds:x}-{length:x}\"");
    let mime = mime_guess::from_path(&download.artifact.filename)
        .first_or_octet_stream()
        .to_string();
    let mut response = (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime),
            (
                header::CONTENT_DISPOSITION,
                disposition(&download.artifact.filename),
            ),
            (header::CONTENT_LENGTH, length.to_string()),
            (header::ACCEPT_RANGES, "bytes".into()),
            (header::ETAG, etag.clone()),
            (header::LAST_MODIFIED, httpdate::fmt_http_date(modified)),
            (
                header::CACHE_CONTROL,
                "private, no-cache, must-revalidate".into(),
            ),
        ],
        Body::empty(),
    )
        .into_response();
    // Visibility and storage access precede all conditional responses.
    let status = if (headers.contains_key(header::IF_MATCH)
        && !matches_tag(&headers, header::IF_MATCH, &etag, false))
        || (!headers.contains_key(header::IF_MATCH)
            && date(&headers, header::IF_UNMODIFIED_SINCE).is_some_and(|d| modified > d))
    {
        Some(StatusCode::PRECONDITION_FAILED)
    } else if matches_tag(&headers, header::IF_NONE_MATCH, &etag, true)
        || (!headers.contains_key(header::IF_NONE_MATCH)
            && date(&headers, header::IF_MODIFIED_SINCE).is_some_and(|d| modified <= d))
    {
        Some(StatusCode::NOT_MODIFIED)
    } else {
        None
    };
    if let Some(status) = status {
        *response.status_mut() = status;
        response.headers_mut().remove(header::CONTENT_LENGTH);
        response.headers_mut().remove(header::CONTENT_DISPOSITION);
        return Ok(response);
    }
    if method == Method::HEAD {
        return Ok(response);
    }
    let if_range = headers.get(header::IF_RANGE).is_none_or(|v| {
        v.to_str()
            .is_ok_and(|s| s == etag || httpdate::parse_http_date(s).is_ok_and(|d| d == modified))
    });
    let requested = if if_range {
        if let Some(value) = headers.get(header::RANGE) {
            match value.to_str().ok().and_then(|s| range(s, length).ok()) {
                Some(r) => r,
                None => {
                    *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
                    response.headers_mut().insert(
                        header::CONTENT_RANGE,
                        format!("bytes */{length}")
                            .parse()
                            .map_err(|_| ApiError::Internal)?,
                    );
                    response.headers_mut().insert(
                        header::CONTENT_LENGTH,
                        "0".parse().map_err(|_| ApiError::Internal)?,
                    );
                    return Ok(response);
                }
            }
        } else {
            None
        }
    } else {
        None
    };
    if state.distribution.config.delivery_mode == DeliveryMode::Nginx {
        // Nginx's range filter works from the full representation. Precondition
        // failures and HEAD have already been handled above without a transfer.
        response.headers_mut().insert(
            "x-accel-redirect",
            state
                .distribution
                .internal_uri(&download.artifact)?
                .parse()
                .map_err(|_| ApiError::Internal)?,
        );
        return Ok(response);
    }
    let mut count = length;
    if let Some((start, end)) = requested {
        count = end - start + 1;
        file.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(|_| ApiError::StorageUnavailable)?;
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{length}")
                .parse()
                .map_err(|_| ApiError::Internal)?,
        );
        response.headers_mut().insert(
            header::CONTENT_LENGTH,
            count.to_string().parse().map_err(|_| ApiError::Internal)?,
        );
    }
    let reader = ReaderStream::with_capacity(file.take(count), 64 * 1024);
    // Stream owns file + permit. Client disconnect drops both immediately.
    let body = stream::unfold((reader, permit), |(mut reader, permit)| async move {
        reader.next().await.map(|chunk| (chunk, (reader, permit)))
    });
    *response.body_mut() = Body::from_stream(body);
    Ok(response)
}

//! Bound multipart parser read-ahead, including hostile preambles/field headers.
//! Multer drains every immediately-ready input chunk in a poll. Yield after each
//! 64 KiB slice and give header parsing a separate budget from file contents.
use super::error::ApiError;
use axum::{
    body::{Body, BodyDataStream, Bytes},
    extract::{FromRequest, Multipart, Request},
};
use futures_util::Stream;
use std::{
    error::Error,
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};
const HEADER_BUDGET: usize = 16 * 1024;
pub(super) struct ParserBudget(Arc<AtomicUsize>);
impl ParserBudget {
    pub fn headers(&self) {
        self.0.store(HEADER_BUDGET, Ordering::Relaxed);
    }
    pub fn file(&self, max: usize) {
        self.0.store(max, Ordering::Relaxed);
    }
}
pub(super) async fn multipart(
    request: Request,
    max_request: usize,
) -> Result<(Multipart, ParserBudget), ApiError> {
    if request
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|v| v > max_request as u64)
    {
        return Err(ApiError::PayloadTooLarge);
    }
    let (parts, body) = request.into_parts();
    let budget = Arc::new(AtomicUsize::new(HEADER_BUDGET));
    let input = Input {
        stream: body.into_data_stream(),
        pending: Bytes::new(),
        yield_next: false,
        budget: budget.clone(),
        failed: false,
    };
    let multipart =
        Multipart::from_request(Request::from_parts(parts, Body::from_stream(input)), &())
            .await
            .map_err(|_| ApiError::InvalidArtifact)?;
    Ok((multipart, ParserBudget(budget)))
}
struct Input {
    stream: BodyDataStream,
    pending: Bytes,
    yield_next: bool,
    budget: Arc<AtomicUsize>,
    failed: bool,
}
impl Stream for Input {
    type Item = Result<Bytes, Box<dyn Error + Send + Sync>>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.failed {
            return Poll::Ready(None);
        }
        if this.yield_next {
            this.yield_next = false;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        if this.pending.is_empty() {
            match Pin::new(&mut this.stream).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => this.pending = bytes,
                Poll::Ready(Some(Err(error))) => {
                    this.failed = true;
                    return Poll::Ready(Some(Err(error.into())));
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
        let allowed = this.budget.load(Ordering::Relaxed);
        if allowed == 0 {
            this.failed = true;
            return Poll::Ready(Some(Err(io::Error::other(
                "Multipart parser budget exceeded",
            )
            .into())));
        }
        let count = this.pending.len().min(64 * 1024).min(allowed);
        this.budget.fetch_sub(count, Ordering::Relaxed);
        this.yield_next = true;
        Poll::Ready(Some(Ok(this.pending.split_to(count))))
    }
}

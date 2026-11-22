use crate::infrastructure::icons::{IconFormat, StoredIcon};
use axum::{
    http::header,
    response::{IntoResponse, Response},
};

pub(super) fn response(icon: StoredIcon) -> Response {
    (
        [
            (header::CONTENT_TYPE, match icon.format {
                IconFormat::Png => "image/png",
                IconFormat::Svg => "image/svg+xml",
            }),
            // Applies even when the image is opened as a top-level document.
            (header::CONTENT_SECURITY_POLICY, "sandbox; default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        icon.bytes,
    ).into_response()
}

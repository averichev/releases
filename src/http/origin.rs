//! Resolve the deployment origin once per request; never persist a request's host.
use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, header, uri::Authority},
    middleware::Next,
    response::Response,
};

use super::{AppState, error::ApiError};
use crate::services::{
    manifest::ManifestGeneration, standalone::StandaloneUpdater, updater::TauriUpdater,
};

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, ApiError> {
    let mut values = headers.get_all(name).iter();
    let value = values
        .next()
        .map(|v| v.to_str().map_err(|_| ApiError::BadRequest))
        .transpose()?;
    if values.next().is_some() {
        return Err(ApiError::BadRequest);
    }
    Ok(value)
}

fn request_origin(request: &Request, fallback: SocketAddr) -> Result<String, ApiError> {
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>();
    // The default listener and My Host's nginx upstream use loopback. A remote
    // client cannot assert HTTPS by sending proxy headers directly.
    let trusted = peer.is_some_and(|ConnectInfo(address)| address.ip().is_loopback());
    let scheme = if trusted {
        match single_header(request.headers(), "x-forwarded-proto")? {
            Some("https") => "https",
            Some("http") | None => "http",
            _ => return Err(ApiError::BadRequest),
        }
    } else {
        "http"
    };
    let host = single_header(request.headers(), header::HOST.as_str())?;
    // Tower tests/internal calls have no network peer. Real HTTP requests must
    // carry an authority, including for health probes.
    let fallback = fallback.to_string();
    let host = host
        .or_else(|| request.uri().authority().map(Authority::as_str))
        .or_else(|| peer.is_none().then_some(fallback.as_str()))
        .ok_or(ApiError::BadRequest)?;
    host.parse::<Authority>()
        .map_err(|_| ApiError::BadRequest)?;
    let url = url::Url::parse(&format!("{scheme}://{host}")).map_err(|_| ApiError::BadRequest)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port() == Some(0)
        || url.host_str().is_none()
    {
        return Err(ApiError::BadRequest);
    }
    let local = matches!(url.host_str(), Some("localhost" | "[::1]"))
        || url
            .host_str()
            .and_then(|h| h.parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
    // Public deployments require TLS at the loopback proxy. This also rejects
    // DNS rebinding to a plain HTTP loopback listener with an attacker domain.
    if scheme == "http" && !local {
        return Err(ApiError::BadRequest);
    }
    Ok(url.origin().ascii_serialization())
}

pub async fn resolve(
    State(mut state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if state.auth_config.public_origin.is_empty()
        || state.distribution.config.public_base_url.is_empty()
    {
        let origin = request_origin(&request, state.server_bind)?;
        if state.auth_config.public_origin.is_empty() {
            state.auth_config.public_origin = origin.clone();
        }
        if state.distribution.config.public_base_url.is_empty() {
            state.distribution.config.public_base_url = origin;
            state.manifests = ManifestGeneration::new(state.distribution.clone());
            state.updater = TauriUpdater::new(state.distribution.clone());
            state.standalone_updater = StandaloneUpdater::new(state.distribution.clone());
            state.tauri_integration = state
                .tauri_integration
                .with_distribution(state.distribution.clone());
        }
    }
    request.extensions_mut().insert(state);
    Ok(next.run(request).await)
}

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use tracing::error;

use crate::services::dash_proxy::{fetch_segment, patch_mpd};
use crate::state::AppState;

const PROXY_BASE: &str = "http://127.0.0.1:3001/dash-proxy/";

pub async fn debug_mpd(State(state): State<AppState>) -> impl IntoResponse {
    let mpd_url = state.source_config.read().await.source_url.clone();

    if mpd_url.is_empty() {
        return (StatusCode::SERVICE_UNAVAILABLE, "MPD URL not yet available").into_response();
    }

    let (origin, referer) = {
        let cfg = state.source_config.read().await;
        (cfg.source_origin.clone(), cfg.source_referer.clone())
    };

    let data = match fetch_segment(
        &state.http_client,
        &mpd_url,
        &state.source_user_agent,
        &origin,
        &referer,
    )
    .await
    {
        Ok(d) => d,
        Err(e) => {
            error!(error = %e, "debug: failed to fetch MPD");
            return (StatusCode::BAD_GATEWAY, e).into_response();
        }
    };

    let raw_xml = String::from_utf8_lossy(&data);
    let patched_xml = patch_mpd(&raw_xml, PROXY_BASE);

    let truncated_url = &mpd_url[..mpd_url.len().min(120)];

    let body = format!(
        "=== MPD URL (truncated) ===\n{}\n\n\
         === RAW MPD ({} bytes) ===\n{}\n\n\
         === PATCHED MPD ({} bytes) ===\n{}",
        truncated_url,
        raw_xml.len(),
        raw_xml,
        patched_xml.len(),
        patched_xml,
    );

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/plain; charset=utf-8".parse().unwrap(),
    );
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());

    (StatusCode::OK, headers, body).into_response()
}

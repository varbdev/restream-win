use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect};

pub async fn stream_playlist() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    (
        StatusCode::TEMPORARY_REDIRECT,
        headers,
        Redirect::temporary("/hls/stream.m3u8"),
    )
        .into_response()
}

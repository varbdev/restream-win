use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use bytes::Bytes;
use tracing::{error, info, warn};

use crate::services::dash_proxy::{
    decrypt_mp4, extract_base_url, fetch_segment, patch_mpd, strip_cenc_from_init,
};
use crate::services::manifest_trimmer::trim_segment_timelines;
use crate::state::AppState;

const WINSPORTS_KID: &str = "19ca642b3eba4f4d81637667a04fdd9e";
const DUMMY_KEY: &str = "00000000000000000000000000000000";
const PROXY_BASE: &str = "http://127.0.0.1:3001/dash-proxy/";
const STALE_VIDEO_THRESHOLD: u64 = 0;
const STALE_AUDIO_THRESHOLD: u64 = 0;
const CDN_RETRY_MAX: u32 = 3;
const CDN_RETRY_DELAY_MS: u64 = 1000;

static INIT_CACHE: OnceLock<Mutex<HashMap<String, Bytes>>> = OnceLock::new();

fn init_cache() -> &'static Mutex<HashMap<String, Bytes>> {
    INIT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn clear_init_cache() {
    if let Ok(mut cache) = init_cache().lock() {
        cache.clear();
    }
}

fn track_key_from_path(segment_path: &str) -> String {
    let base = segment_path
        .split('?')
        .next()
        .unwrap_or(segment_path)
        .trim_end_matches(".mp4");

    if base.ends_with("_init") {
        return base.trim_end_matches("_init").to_string();
    }

    if let Some(pos) = base.rfind('_') {
        let suffix = &base[pos + 1..];
        if !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
            return base[..pos].to_string();
        }
    }

    base.to_string()
}

fn store_raw_init(track: &str, data: Bytes) {
    if let Ok(mut cache) = init_cache().lock() {
        cache.insert(track.to_string(), data);
    }
}

fn get_raw_init(track: &str) -> Option<Bytes> {
    init_cache().lock().ok()?.get(track).cloned()
}

fn extract_segment_seq(cache_key: &str) -> Option<u64> {
    let base = cache_key.trim_end_matches(".mp4");
    if base.contains("_init") {
        return None;
    }
    let pos = base.rfind('_')?;
    let suffix = &base[pos + 1..];
    if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let num: u64 = suffix.parse().ok()?;
    if num < 1000 {
        return None;
    }
    Some(num)
}

fn reject_if_stale(state: &AppState, cache_key: &str, is_video: bool) -> bool {
    let Some(seq) = extract_segment_seq(cache_key) else {
        return false;
    };

    let (counter, threshold, label) = if is_video {
        (&state.max_video_seq, STALE_VIDEO_THRESHOLD, "video")
    } else {
        (&state.max_audio_seq, STALE_AUDIO_THRESHOLD, "audio")
    };

    let current_max = counter.load(Ordering::Relaxed);

    if current_max > seq.saturating_add(threshold) {
        warn!(
            path = %cache_key,
            seq,
            current_max,
            stream_type = label,
            "stale segment rejected to prevent DTS regression"
        );
        return true;
    }

    counter.fetch_max(seq, Ordering::Relaxed);
    false
}

async fn fetch_cdn_segment_with_retry(
    state: &AppState,
    segment_path: &str,
    query_str: &str,
) -> Result<Bytes, String> {
    let mut last_err = String::new();

    for attempt in 0..=CDN_RETRY_MAX {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(CDN_RETRY_DELAY_MS)).await;
        }

        let mpd_url = state.source_config.read().await.source_url.clone();
        let base_url = extract_base_url(&mpd_url);
        let cdn_url = format!("{}{}{}", base_url, segment_path, query_str);

        let (origin, referer) = {
            let cfg = state.source_config.read().await;
            (cfg.source_origin.clone(), cfg.source_referer.clone())
        };

        info!(
            path = %segment_path,
            url = %&cdn_url[..cdn_url.len().min(80)],
            attempt,
            "proxy fetching segment"
        );

        match fetch_segment(
            &state.http_client,
            &cdn_url,
            &state.source_user_agent,
            &origin,
            &referer,
        )
        .await
        {
            Ok(data) => {
                info!(
                    path = %segment_path,
                    bytes = data.len(),
                    attempt,
                    "segment fetched from CDN"
                );
                return Ok(data);
            }
            Err(e) if e.contains("404") && attempt < CDN_RETRY_MAX => {
                warn!(
                    path = %segment_path,
                    url = %cdn_url,
                    attempt,
                    error = %e,
                    "CDN segment not yet available, retrying"
                );
                last_err = e;
            }
            Err(e) => {
                error!(
                    path = %segment_path,
                    url = %cdn_url,
                    attempt,
                    error = %e,
                    "failed to fetch segment from CDN"
                );
                return Err(e);
            }
        }
    }

    error!(
        path = %segment_path,
        "CDN segment 404 after all retries"
    );
    Err(last_err)
}

pub async fn proxy_manifest(State(state): State<AppState>) -> impl IntoResponse {
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
            error!(error = %e, "failed to fetch MPD for proxy");
            return (StatusCode::SERVICE_UNAVAILABLE, e).into_response();
        }
    };

    let mpd_xml = String::from_utf8_lossy(&data);
    let min_video_seq = state
        .max_video_seq
        .load(Ordering::Relaxed)
        .saturating_sub(1);
    let min_audio_seq = state
        .max_audio_seq
        .load(Ordering::Relaxed)
        .saturating_sub(1);
    let trimmed = trim_segment_timelines(&mpd_xml, min_video_seq, min_audio_seq);
    let patched = patch_mpd(&trimmed, PROXY_BASE);

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "application/dash+xml".parse().unwrap(),
    );
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());

    (StatusCode::OK, headers, patched).into_response()
}

pub async fn proxy_segment(
    State(state): State<AppState>,
    Path(segment_path): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let mpd_url = state.source_config.read().await.source_url.clone();

    if mpd_url.is_empty() {
        return (StatusCode::SERVICE_UNAVAILABLE, "MPD URL not available").into_response();
    }

    let cache_key = segment_path
        .split('?')
        .next()
        .unwrap_or(&segment_path)
        .to_string();

    let is_init = segment_path.contains("init");
    let is_video = cache_key.contains("video");

    if !is_init && reject_if_stale(&state, &cache_key, is_video) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }

    let query_str = if params.is_empty() {
        String::new()
    } else {
        let pairs: Vec<String> = params.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
        format!("?{}", pairs.join("&"))
    };

    let data = match fetch_cdn_segment_with_retry(&state, &segment_path, &query_str).await {
        Ok(d) => d,
        Err(e) => {
            return (StatusCode::SERVICE_UNAVAILABLE, e).into_response();
        }
    };

    let decryption_key = state.decryption_key.read().await.clone();
    let track = track_key_from_path(&segment_path);

    let final_data = if is_init {
        store_raw_init(&track, data.clone());

        let key = decryption_key.as_deref().unwrap_or(DUMMY_KEY);
        match strip_cenc_from_init(data, WINSPORTS_KID, key).await {
            Ok(stripped) => {
                info!(
                    path = %segment_path,
                    bytes = stripped.len(),
                    "init segment stripped of CENC metadata"
                );
                stripped
            }
            Err(e) => {
                error!(error = %e, path = %segment_path, "mp4decrypt failed on init segment");
                return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
            }
        }
    } else if let Some(key) = decryption_key {
        match get_raw_init(&track) {
            Some(raw_init) => match decrypt_mp4(data, &raw_init, WINSPORTS_KID, &key).await {
                Ok(decrypted) => {
                    info!(
                        path = %segment_path,
                        bytes = decrypted.len(),
                        "media segment decrypted"
                    );
                    decrypted
                }
                Err(e) => {
                    error!(error = %e, path = %segment_path, "mp4decrypt failed on media segment");
                    return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
                }
            },
            None => {
                warn!(
                    path = %segment_path,
                    track = %track,
                    "no cached init for track — passing raw encrypted segment"
                );
                data
            }
        }
    } else {
        info!(
            path = %segment_path,
            bytes = data.len(),
            "no key available — passing raw encrypted segment"
        );
        data
    };

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "video/mp4".parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());

    (StatusCode::OK, headers, final_data.to_vec()).into_response()
}

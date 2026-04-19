use axum::extract::{Form, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect};
use serde::Deserialize;
use tracing::{info, warn};

use crate::state::{AppState, SourceConfig};
use crate::utils::escape::escape_html;

#[derive(Deserialize)]
pub struct AdminUpdateForm {
    pub source_url: String,
    pub source_origin: String,
    pub source_referer: String,
    pub jwt_token: String,
    pub decryption_key: String,
}

fn mask_secret(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let visible_len = 8.min(value.len());
    format!("{}…", &value[..visible_len])
}

pub async fn admin_page(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.source_config.read().await;
    let jwt = state.jwt_token.read().await;
    let key = state.decryption_key.read().await;

    let jwt_masked = mask_secret(jwt.as_str());
    let key_masked = key.as_deref().map(mask_secret).unwrap_or_default();

    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Restream DASH Admin</title>
  <style>
    body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; margin: 2rem; color: #111; }}
    .card {{ max-width: 900px; border: 1px solid #ddd; border-radius: 8px; padding: 1rem; }}
    label {{ display: block; margin: .75rem 0 .25rem; font-weight: 600; }}
    input, textarea {{ width: 100%; box-sizing: border-box; padding: .6rem; border: 1px solid #bbb; border-radius: 6px; font-size: 14px; }}
    .current {{ font-size: 13px; color: #555; margin: .2rem 0 .4rem; font-family: monospace; }}
    button {{ margin-top: 1rem; padding: .6rem 1rem; border: 0; border-radius: 6px; background: #0b57d0; color: #fff; cursor: pointer; }}
    .hint {{ margin-top: .75rem; font-size: 13px; color: #555; }}
    hr {{ margin: 1.5rem 0; border: none; border-top: 1px solid #eee; }}
  </style>
</head>
<body>
  <div class="card">
    <h1>Restream DASH Admin</h1>
    <form method="post" action="/admin/source">
      <label for="source_url">SOURCE_URL (DASH .mpd)</label>
      <textarea id="source_url" name="source_url" rows="5" required>{}</textarea>

      <label for="source_origin">SOURCE_ORIGIN</label>
      <input id="source_origin" name="source_origin" type="text" required value="{}" />

      <label for="source_referer">SOURCE_REFERER</label>
      <input id="source_referer" name="source_referer" type="text" required value="{}" />

      <hr />

      <label for="jwt_token">JWT_TOKEN</label>
      {}
      <input id="jwt_token" name="jwt_token" type="password" autocomplete="off" placeholder="Enter new token to replace…" value="" />

      <label for="decryption_key">DECRYPTION_KEY</label>
      {}
      <input id="decryption_key" name="decryption_key" type="password" autocomplete="off" placeholder="Enter new key to replace…" value="" />

      <button type="submit">Save and Restart Worker</button>
    </form>
    <p class="hint">Leave JWT_TOKEN or DECRYPTION_KEY blank to keep the current value. Saving restarts ffmpeg and deletes old segments.</p>
  </div>
</body>
</html>"#,
        escape_html(&config.source_url),
        escape_html(&config.source_origin),
        escape_html(&config.source_referer),
        if jwt_masked.is_empty() {
            "<p class=\"current\">not set</p>".to_string()
        } else {
            format!(
                "<p class=\"current\">current: {}</p>",
                escape_html(&jwt_masked)
            )
        },
        if key_masked.is_empty() {
            "<p class=\"current\">not set</p>".to_string()
        } else {
            format!(
                "<p class=\"current\">current: {}</p>",
                escape_html(&key_masked)
            )
        },
    );

    Html(html)
}

pub async fn update_source(
    State(state): State<AppState>,
    Form(form): Form<AdminUpdateForm>,
) -> impl IntoResponse {
    let source_url = form.source_url.trim().to_string();
    let source_origin = form.source_origin.trim().to_string();
    let source_referer = form.source_referer.trim().to_string();

    if source_url.is_empty() || source_origin.is_empty() || source_referer.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "source_url, source_origin and source_referer are required",
        )
            .into_response();
    }

    {
        let mut cfg = state.source_config.write().await;
        *cfg = SourceConfig {
            source_url,
            source_origin,
            source_referer,
        };
    }

    let new_jwt = form.jwt_token.trim().to_string();
    if !new_jwt.is_empty() {
        let mut jwt = state.jwt_token.write().await;
        *jwt = new_jwt;
        info!("jwt_token updated via admin");
    }

    let new_key = form.decryption_key.trim().to_string();
    if !new_key.is_empty() {
        let mut key = state.decryption_key.write().await;
        *key = Some(new_key);
        info!("decryption_key updated via admin");
    }

    if let Err(err) = state.restart_tx.send(()) {
        warn!(error = %err, "failed to send ffmpeg restart signal");
    } else {
        info!("source config updated; requested ffmpeg restart");
    }

    Redirect::to("/admin").into_response()
}
